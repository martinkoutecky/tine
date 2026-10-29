//! The graph's owner (CLI PROTOCOL, `tine-agents/specs/campaigns/2026-10-cli/
//! PROTOCOL.md`): the one process running a graph's page host, the app or a
//! headless `tine serve`, holding the graph's owner lock and answering the
//! command line on a per-user endpoint. The command line is a client of the
//! owner; it never opens a page host of its own.
//!
//! - `namespace`: where the lock and endpoint live, derived from the account;
//! - `lock`: the owner lock;
//! - `wire`: framing and messages;
//! - `endpoint`: the listener, its connections and its admission gate;
//! - `lease`: lock plus endpoint, kept with the host (the app's `GraphSlot`);
//! - `verbs`: the read verbs over a `GraphSlot`, shared by both owners;
//! - `serve`, `client` and `terminal`: the command line's side (desktop
//!   only): the headless owner, discovery and handover, the read commands.

mod add;
pub(crate) mod arena;
#[cfg(desktop)]
pub(crate) mod client;
pub(crate) mod endpoint;
pub(crate) mod git_autocommit;
pub(crate) mod git_identity;
pub(crate) mod git_pull;
pub(crate) mod git_run;
pub(crate) mod git_sync;
pub(crate) mod git_sync_cycle;
pub(crate) mod lease;
pub(crate) mod lock;
pub(crate) mod namespace;
mod pages;
pub(crate) mod seen;
#[cfg(desktop)]
pub(crate) mod serve;
#[cfg(desktop)]
pub(crate) mod terminal;
pub(crate) mod verbs;
pub(crate) mod window;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
mod windows_security;
pub(crate) mod wire;
pub(crate) mod write;

// Both exercise `serve`, `client` and `terminal`, which are desktop only.
#[cfg(all(test, desktop))]
mod output_tests;
#[cfg(all(test, desktop))]
mod owner_tests;

/// A failure to reach or become the owner (PROTOCOL §2.3, client-side codes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OwnerError {
    /// The namespace directory or endpoint path is not safe to use (§1).
    Unsafe(String),
    /// The lock call itself failed; not contention (§3.5).
    Lock(String),
    /// `tine serve` did not come up, or the owner's endpoint does not answer.
    Start(String),
    /// The owner speaks another protocol, or is not this graph's owner.
    Protocol(String),
    Mismatch(String),
    /// No command line (iOS, Android), or an unshipped target.
    #[cfg_attr(
        any(target_os = "linux", target_os = "macos", target_os = "windows"),
        allow(dead_code)
    )]
    Unsupported,
}

impl OwnerError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Unsafe(_) => "owner-unsafe",
            Self::Lock(_) => "owner-lock",
            Self::Start(_) => "owner-start",
            Self::Protocol(_) => "protocol",
            Self::Mismatch(_) => "owner-mismatch",
            Self::Unsupported => "unsupported",
        }
    }
}

impl std::fmt::Display for OwnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsafe(m)
            | Self::Lock(m)
            | Self::Start(m)
            | Self::Protocol(m)
            | Self::Mismatch(m) => f.write_str(m),
            Self::Unsupported => {
                f.write_str("the command line cannot reach a graph's owner on this platform")
            }
        }
    }
}

#[cfg(test)]
mod platform_gate_tests {
    use std::path::Path;

    /// A test module that reaches a desktop-only owner module is itself
    /// desktop only, else `cargo check --tests` fails for Android and iOS.
    #[test]
    fn test_modules_that_use_desktop_only_modules_are_desktop_gated() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/owner");
        let source = include_str!("mod.rs");
        let mut cfg = "";
        let mut desktop_only = Vec::new();
        let mut test_modules = Vec::new();
        for line in source.lines().map(str::trim) {
            if line.starts_with("#[cfg") {
                cfg = line;
                continue;
            }
            let Some(name) = line
                .strip_prefix("pub(crate) mod ")
                .or_else(|| line.strip_prefix("mod "))
                .and_then(|rest| rest.strip_suffix(';'))
            else {
                cfg = "";
                continue;
            };
            if cfg == "#[cfg(desktop)]" {
                desktop_only.push(name);
            } else if cfg.contains("test") {
                test_modules.push((name, cfg));
            }
            cfg = "";
        }
        assert!(desktop_only.contains(&"serve"), "{desktop_only:?}");
        assert!(test_modules.iter().any(|(name, _)| *name == "owner_tests"));
        for (name, cfg) in test_modules {
            let mut text =
                std::fs::read_to_string(dir.join(format!("{name}.rs"))).unwrap_or_default();
            if let Ok(entries) = std::fs::read_dir(dir.join(name)) {
                for entry in entries.flatten() {
                    if entry.path().extension().is_some_and(|ext| ext == "rs") {
                        text += &std::fs::read_to_string(entry.path()).unwrap();
                    }
                }
            }
            assert!(!text.is_empty(), "{name}: no source found");
            let uses = desktop_only
                .iter()
                .find(|module| text.contains(&format!("{module}::")));
            if let Some(module) = uses {
                assert!(
                    cfg.contains("desktop"),
                    "owner::{name} uses the desktop-only `{module}` but is gated `{cfg}`"
                );
            }
        }
    }
}
