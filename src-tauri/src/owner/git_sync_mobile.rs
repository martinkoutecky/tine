//! ADR 0078 slice 3 owns mobile Git. No git2 dependency or transport on mobile.
#![allow(dead_code)]
pub(crate) const AVAILABLE: bool = false;
pub(crate) const UNAVAILABLE_REASON: &str = "Git is not available on this device yet";
