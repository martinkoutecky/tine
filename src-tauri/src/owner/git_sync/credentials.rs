//! Git HTTPS tokens on this device (ADR 0078 §5), one per remote origin
//! (`https://host[:port]`) with the username it was given for. Kept in the
//! private app-data file `git-credentials.json`, like quick share's account
//! file (ADR 0076): written only through the owner-only atomic writer
//! (`device_io::atomic_write_private`), never in a graph, the repository
//! config, the webview's settings file, a log or an error. Storage choice:
//! GIT-CRED (open for Martin); this module is the only reader and writer, so
//! an OS keychain could replace the file without touching its callers.
//!
//! Refusal (contract: ADR 0078 §5): an unreadable or newer-version file reads
//! as signed out (threat: disk error, torn write by another tool, an external
//! editor, a newer Tine on the same app data); signing in writes a fresh file.
//! Tokens are re-obtainable from the host, so nothing exists only here.

use std::io;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::{Credentials, Token};
use crate::device_io::{atomic_write_private, read_app_text};

pub(crate) const FILE: &str = "git-credentials.json";
const VERSION: u32 = 1;
const MAX_USERNAME: usize = 256;
const MAX_TOKEN: usize = 4096;

/// Every read-modify-write of the file holds this (every window shares the
/// process).
static LOCK: Mutex<()> = Mutex::new(());

#[derive(Serialize, Deserialize)]
struct TokenFile {
    v: u32,
    accounts: Vec<Account>,
}

#[derive(Serialize, Deserialize)]
struct Account {
    origin: String,
    username: String,
    token: String,
    signed_in_at: i64,
}

/// This device's git tokens, in the app-data folder `dir`.
#[derive(Clone)]
pub(crate) struct Tokens {
    dir: PathBuf,
}

impl Tokens {
    pub(crate) fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self) -> PathBuf {
        self.dir.join(FILE)
    }

    fn accounts(&self) -> Vec<Account> {
        read_app_text(&self.path())
            .ok()
            .and_then(|text| serde_json::from_str::<TokenFile>(&text).ok())
            .filter(|file| file.v == VERSION)
            .map(|file| file.accounts)
            .unwrap_or_default()
    }

    fn write(&self, accounts: Vec<Account>) -> io::Result<()> {
        if accounts.is_empty() {
            return match std::fs::remove_file(self.path()) {
                Ok(()) => tine_store::directory_durability::sync_directory_entry(&self.dir),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            };
        }
        let bytes = serde_json::to_vec(&TokenFile {
            v: VERSION,
            accounts,
        })
        .map_err(io::Error::other)?;
        std::fs::create_dir_all(&self.dir)?;
        atomic_write_private(&self.path(), &bytes)
    }

    /// The username signed in for `origin`, if any.
    pub(crate) fn signed_in(&self, origin: &str) -> Option<String> {
        self.accounts()
            .into_iter()
            .find(|a| a.origin == origin)
            .map(|a| a.username)
    }

    /// Keep `token` for `origin`, replacing an earlier sign-in there. The
    /// error never contains the token.
    pub(crate) fn sign_in(&self, origin: &str, username: &str, token: &str) -> Result<(), String> {
        let username = username.trim();
        let token = token.trim();
        if username.is_empty()
            || username.len() > MAX_USERNAME
            || username.chars().any(char::is_control)
        {
            return Err("Enter the username you sign in to the git host with.".into());
        }
        if token.is_empty()
            || token.len() > MAX_TOKEN
            || token.chars().any(|c| c.is_control() || c.is_whitespace())
        {
            return Err("Paste the whole token, without spaces.".into());
        }
        let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let mut accounts = self.accounts();
        accounts.retain(|a| a.origin != origin);
        accounts.push(Account {
            origin: origin.to_string(),
            username: username.to_string(),
            token: token.to_string(),
            signed_in_at: chrono::Utc::now().timestamp(),
        });
        self.write(accounts)
            .map_err(|_| "Tine could not save the sign-in on this device.".to_owned())
    }

    /// Forget the token for `origin`.
    pub(crate) fn sign_out(&self, origin: &str) -> Result<(), String> {
        let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let mut accounts = self.accounts();
        let before = accounts.len();
        accounts.retain(|a| a.origin != origin);
        if accounts.len() == before {
            return Ok(());
        }
        self.write(accounts)
            .map_err(|_| "Tine could not remove the sign-in on this device.".to_owned())
    }
}

impl Credentials for Tokens {
    fn token(&self, repository_url: &str) -> Option<Token> {
        let origin = super::https_origin(repository_url)?;
        self.accounts()
            .into_iter()
            .find(|a| a.origin == origin)
            .map(|a| Token {
                username: a.username,
                secret: a.token,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_kept_per_origin_owner_only_and_never_in_errors() {
        let dir = tempfile::tempdir().unwrap();
        let tokens = Tokens::new(dir.path().join("app"));
        assert!(tokens.token("https://example.org/me/notes.git").is_none());
        tokens
            .sign_in("https://example.org", "me", " secret-1 ")
            .unwrap();
        tokens
            .sign_in("https://git.example.net:8443", "other", "secret-2")
            .unwrap();
        let token = tokens.token("https://example.org/me/notes.git").unwrap();
        assert_eq!(
            (token.username.as_str(), token.secret.as_str()),
            ("me", "secret-1")
        );
        assert_eq!(
            tokens
                .token("https://git.example.net:8443/x.git")
                .unwrap()
                .secret,
            "secret-2"
        );
        // Another port, another scheme or a plain path never gets a token.
        assert!(tokens.token("https://git.example.net/x.git").is_none());
        assert!(tokens.token("http://example.org/me/notes.git").is_none());
        assert!(tokens.token("file:///tmp/remote.git").is_none());
        assert_eq!(
            tokens.signed_in("https://example.org").as_deref(),
            Some("me")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("app").join(FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "owner-only from creation");
        }
        // Signing in again replaces the token for that origin only.
        tokens
            .sign_in("https://example.org", "me2", "secret-3")
            .unwrap();
        assert_eq!(
            tokens.token("https://example.org/a.git").unwrap().secret,
            "secret-3"
        );
        let refused = tokens
            .sign_in("https://example.org", "me", "has space")
            .unwrap_err();
        assert!(!refused.contains("has space"));
        assert!(tokens.sign_in("https://example.org", "", "t").is_err());
        tokens.sign_out("https://example.org").unwrap();
        assert!(tokens.token("https://example.org/a.git").is_none());
        tokens.sign_out("https://git.example.net:8443").unwrap();
        assert!(
            !dir.path().join("app").join(FILE).exists(),
            "the last sign-out deletes the file"
        );
        tokens.sign_out("https://example.org").unwrap();
    }

    #[test]
    fn an_unreadable_or_newer_file_reads_as_signed_out_and_sign_in_replaces_it() {
        let dir = tempfile::tempdir().unwrap();
        let tokens = Tokens::new(dir.path());
        std::fs::write(dir.path().join(FILE), b"{not json").unwrap();
        assert!(tokens.token("https://example.org/a.git").is_none());
        std::fs::write(
            dir.path().join(FILE),
            br#"{"v":2,"accounts":[{"origin":"https://example.org","username":"u","token":"t","signed_in_at":0}]}"#,
        )
        .unwrap();
        assert!(tokens.signed_in("https://example.org").is_none());
        tokens.sign_in("https://example.org", "u", "fresh").unwrap();
        assert_eq!(
            tokens.token("https://example.org/a.git").unwrap().secret,
            "fresh"
        );
    }
}
