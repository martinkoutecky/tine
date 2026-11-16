//! Who Tine commits the user's changes as, and whom seen treats as "me"
//! (ADR 0078 §2, ADR 0079). Split from `git_run.rs` along this seam.

use git2::{Repository, Signature};

use super::git_run::USER_FALLBACK;

/// The identity Tine's git writers commit the user's changes as (autocommits,
/// a run's snapshot of the user's pending bytes), and therefore the one seen
/// treats as "me" (ADR 0078 §2, ADR 0079): the repository's `user.name` /
/// `user.email` (local, global or system config) where configured; otherwise
/// "Your name" as the name with the placeholder email; only when neither
/// exists the fixed [`USER_FALLBACK`]. One function, so the writer and the
/// reader cannot disagree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UserIdentity {
    pub(crate) name: String,
    pub(crate) email: String,
    /// False only for the fixed fallback, which every unconfigured Tine shares.
    pub(crate) known: bool,
}

pub(crate) fn user_identity(repo: &Repository, your_name: &str) -> UserIdentity {
    let config = repo.config().ok();
    let get = |key: &str| {
        config
            .as_ref()
            .and_then(|c| c.get_string(key).ok())
            .map(|v| v.trim().to_owned())
    };
    identity_from(get("user.name"), get("user.email"), your_name)
}

/// [`user_identity`] over already-read values. A value libgit2 would refuse
/// in a signature (empty, or holding `<`/`>`) counts as unset.
pub(crate) fn identity_from(
    repo_name: Option<String>,
    repo_email: Option<String>,
    your_name: &str,
) -> UserIdentity {
    let usable_name = |n: &str| !n.is_empty() && Signature::now(n, USER_FALLBACK.1).is_ok();
    let usable_email = |e: &str| !e.is_empty() && Signature::now(USER_FALLBACK.0, e).is_ok();
    let your_name = your_name.trim();
    let name = repo_name
        .filter(|n| usable_name(n))
        .or_else(|| usable_name(your_name).then(|| your_name.to_owned()));
    let email = repo_email.filter(|e| usable_email(e));
    UserIdentity {
        known: name.is_some() || email.is_some(),
        name: name.unwrap_or_else(|| USER_FALLBACK.0.to_owned()),
        email: email.unwrap_or_else(|| USER_FALLBACK.1.to_owned()),
    }
}

impl UserIdentity {
    pub(crate) fn signature(&self) -> Signature<'static> {
        Signature::now(&self.name, &self.email).expect("validated identity")
    }

    /// Whether `author` is a commit this identity wrote. A configured email
    /// identifies the user on every machine and tool (case-insensitively, as
    /// git hosts compare it); the shared placeholder email does not, so then
    /// the name must match exactly too.
    pub(crate) fn wrote(&self, author: &Signature<'_>) -> bool {
        let Ok(email) = author.email() else {
            return false;
        };
        if self.email != USER_FALLBACK.1 {
            email.eq_ignore_ascii_case(&self.email)
        } else {
            email == self.email && author.name().ok() == Some(self.name.as_str())
        }
    }
}
