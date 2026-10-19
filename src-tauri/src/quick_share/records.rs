//! Quick share's device-local records (DESIGN §1.5): the connected account
//! and this device's shares. App data, never in a graph. Both files hold
//! secrets (the token; each share's key), so every write goes through the
//! private atomic writer (owner-only temp + fsync + rename) under the one
//! process-wide [`RECORDS_LOCK`]; every window shares this process.
//!
//! Refusals (contract `docs/contracts/quick-share.md`, "Records"):
//! - `quick-shares.json` that does not parse, or has a newer `v`, is never
//!   overwritten (threat: disk error, external editor, a newer Tine on the
//!   same app data). It holds keys that exist nowhere else; reads report it
//!   and every share change refuses until it is readable.
//! - An unreadable `quick-share-account.json` reads as not connected
//!   (recovery: Connect writes a fresh one; the token is re-obtainable).

use super::Token;
use crate::device_io::{atomic_write_private, read_app_text};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;

pub(crate) const ACCOUNT_FILE: &str = "quick-share-account.json";
pub(crate) const SHARES_FILE: &str = "quick-shares.json";
const VERSION: u32 = 1;

/// Every record read-modify-write holds this (DESIGN §3.5).
pub(crate) static RECORDS_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn lock() -> std::sync::MutexGuard<'static, ()> {
    RECORDS_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The connected account. `token` is private to this module and the
/// transport; `Debug` prints it redacted.
#[derive(Debug, Clone)]
pub(crate) struct Account {
    pub login: String,
    pub account_id: u64,
    pub token: Token,
    pub scopes: Vec<String>,
    pub connected_at: String,
}

#[derive(Serialize, Deserialize)]
struct AccountFile {
    v: u32,
    login: String,
    account_id: u64,
    token: String,
    scopes: Vec<String>,
    connected_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ShareState {
    Pending,
    Live,
}

/// One share made on this device. `key` (canonical base64url) never leaves
/// Rust except inside the link Tine copies to the clipboard.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ShareRow {
    pub share_id: String,
    pub state: ShareState,
    pub account_id: u64,
    pub login: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gist_id: Option<String>,
    pub created_at: String,
    pub title: String,
    /// Number of shared pages.
    pub pages: usize,
    pub key: String,
}

impl std::fmt::Debug for ShareRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShareRow")
            .field("share_id", &self.share_id)
            .field("state", &self.state)
            .field("gist_id", &self.gist_id)
            .field("key", &"<redacted>")
            .finish_non_exhaustive()
    }
}

#[derive(Serialize, Deserialize)]
struct SharesFile {
    v: u32,
    shares: Vec<ShareRow>,
}

#[derive(Clone)]
pub(crate) struct Records {
    dir: PathBuf,
}

/// The shares file could not be read safely; nothing is written over it.
pub(crate) const SHARES_UNREADABLE: &str =
    "Tine cannot read its list of shares (quick-shares.json); nothing was changed";

impl Records {
    pub(crate) fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        atomic_write_private(&self.path(name), bytes)
    }

    pub(crate) fn account(&self) -> Option<Account> {
        let text = read_app_text(&self.path(ACCOUNT_FILE)).ok()?;
        let file: AccountFile = serde_json::from_str(&text).ok()?;
        (file.v == VERSION && !file.token.is_empty()).then(|| Account {
            login: file.login,
            account_id: file.account_id,
            token: Token::new(file.token),
            scopes: file.scopes,
            connected_at: file.connected_at,
        })
    }

    /// Callers hold [`lock`].
    pub(crate) fn store_account(&self, account: &Account) -> io::Result<()> {
        let file = AccountFile {
            v: VERSION,
            login: account.login.clone(),
            account_id: account.account_id,
            token: account.token.expose().to_owned(),
            scopes: account.scopes.clone(),
            connected_at: account.connected_at.clone(),
        };
        self.write(
            ACCOUNT_FILE,
            &serde_json::to_vec(&file).map_err(io::Error::other)?,
        )
    }

    /// Callers hold [`lock`].
    pub(crate) fn remove_account(&self) -> io::Result<()> {
        match std::fs::remove_file(self.path(ACCOUNT_FILE)) {
            Ok(()) => tine_store::directory_durability::sync_directory_entry(&self.dir),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn shares(&self) -> Result<Vec<ShareRow>, String> {
        let text = match read_app_text(&self.path(SHARES_FILE)) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err(SHARES_UNREADABLE.into()),
        };
        match serde_json::from_str::<SharesFile>(&text) {
            Ok(file) if file.v == VERSION => Ok(file.shares),
            _ => Err(SHARES_UNREADABLE.into()),
        }
    }

    /// Read-modify-write the shares under [`lock`]; `edit` returns whether
    /// anything changed. An unreadable file is never replaced.
    pub(crate) fn update_shares<T>(
        &self,
        edit: impl FnOnce(&mut Vec<ShareRow>) -> T,
    ) -> Result<T, String> {
        let _guard = lock();
        let mut shares = self.shares()?;
        let before = shares.clone();
        let result = edit(&mut shares);
        if shares != before {
            let bytes = serde_json::to_vec(&SharesFile { v: VERSION, shares })
                .map_err(|e| e.to_string())?;
            self.write(SHARES_FILE, &bytes)
                .map_err(|_| "Tine could not save its list of shares".to_owned())?;
        }
        Ok(result)
    }
}
