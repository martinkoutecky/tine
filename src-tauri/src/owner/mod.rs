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

#[cfg(desktop)]
pub(crate) mod client;
pub(crate) mod endpoint;
pub(crate) mod lease;
pub(crate) mod lock;
pub(crate) mod namespace;
#[cfg(desktop)]
pub(crate) mod serve;
#[cfg(desktop)]
pub(crate) mod terminal;
pub(crate) mod verbs;
pub(crate) mod wire;

#[cfg(test)]
mod output_tests;
#[cfg(test)]
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
    /// No owner endpoint on this platform yet (Windows, slice 1a), or none
    /// at all (iOS, Android: no command line). Built only on those targets.
    #[cfg_attr(any(target_os = "linux", target_os = "macos"), allow(dead_code))]
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
                f.write_str("the command line cannot reach a graph's owner on this platform yet")
            }
        }
    }
}
