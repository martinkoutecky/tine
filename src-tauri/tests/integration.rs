//! One integration executable; module names retain each source suite in test filters.

#[path = "youtube_identity.rs"]
mod youtube_identity;

#[cfg(windows)]
#[path = "windows_cli.rs"]
mod windows_cli;
