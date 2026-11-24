//! Which scheme route an authored destination takes: the one rule, read by
//! the link-target owner (`local_target`) and, through wasm, by the
//! renderer's scheme policy (`src/linkSchemePolicy.ts`). The native handoff
//! gate (`src-tauri/src/platform.rs`) rechecks every OS handoff separately.
//!
//! Read on the authored text before any decoding (a scheme is never spelled
//! with escapes). One-letter prefixes name a drive. Pure, O(destination
//! bytes), no I/O.

/// The route a destination's scheme selects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemeRoute {
    /// No scheme: a relative path, fragment or attachment.
    Relative,
    Web,
    Mail,
    /// A literal `file:` prefix (after trimming).
    File,
    /// A one-letter prefix: a Windows drive.
    Drive,
    /// A custom scheme, left to the consent prompt.
    Defer(String),
    /// A refused scheme, including one hidden by control bytes or spaces.
    Drop,
}

/// How a raw HTML resource (`src`, `poster`) may load in the browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewResource {
    /// No scheme (or `file:`): a local path for the local resolver.
    LocalCandidate,
    /// Web or `data:`, loaded by the browser.
    Browser,
    Drop,
}

const REFUSED_SCHEMES: &str = include_str!("../../../src-tauri/external-link-schemes.json");

/// A refused name is one of the shared list's quoted strings. O(list bytes).
pub fn refused_scheme(scheme: &str) -> bool {
    let s = scheme.as_bytes();
    !s.is_empty()
        && REFUSED_SCHEMES
            .as_bytes()
            .windows(s.len() + 2)
            .any(|w| w[0] == b'"' && w[s.len() + 1] == b'"' && &w[1..=s.len()] == s)
}

/// The leading scheme (`Some(name)`, lowercased) or drive letter (`Some("")`),
/// read the way a browser does: leading C0 controls and spaces dropped, tabs
/// and line breaks ignored; `compact` ignores every control and space.
/// O(bytes); byte loops rather than `str` patterns keep the wasm module small.
fn scheme_prefix(dest: &str, compact: bool) -> Option<String> {
    let mut name = String::new();
    for &c in dest.as_bytes() {
        if c <= b' ' && (compact || name.is_empty() || matches!(c, b'\t' | b'\n' | b'\r')) {
            continue;
        }
        if c == b':' && !name.is_empty() {
            return Some(if name.len() == 1 { String::new() } else { name });
        }
        let valid = if name.is_empty() {
            c.is_ascii_alphabetic()
        } else {
            c.is_ascii_alphanumeric() || matches!(c, b'+' | b'.' | b'-')
        };
        if !valid {
            return None;
        }
        name.push(c.to_ascii_lowercase() as char);
    }
    None
}

/// The lowercased URL scheme, `None` for no scheme or a drive letter. O(bytes).
pub fn link_scheme(dest: &str) -> Option<String> {
    scheme_prefix(dest, false).filter(|s| !s.is_empty())
}

/// The scheme route of an authored destination. O(bytes).
pub fn scheme_route(dest: &str) -> SchemeRoute {
    let Some(scheme) = scheme_prefix(dest, false) else {
        // A scheme hidden by embedded control bytes or spaces is still refused.
        return match scheme_prefix(dest, true) {
            Some(s) if refused_scheme(&s) => SchemeRoute::Drop,
            _ => SchemeRoute::Relative,
        };
    };
    match scheme.as_str() {
        "" => SchemeRoute::Drive,
        s if refused_scheme(s) => SchemeRoute::Drop,
        "http" | "https" => SchemeRoute::Web,
        "mailto" => SchemeRoute::Mail,
        "file"
            if crate::local_target::js_trim(dest)
                .get(..5)
                .is_some_and(|p| p.eq_ignore_ascii_case("file:")) =>
        {
            SchemeRoute::File
        }
        _ => SchemeRoute::Defer(scheme),
    }
}

/// The scheme route of a raw HTML resource attribute, every control byte
/// and space ignored. O(bytes).
pub fn preview_resource(dest: &str) -> PreviewResource {
    match scheme_prefix(dest, true).as_deref() {
        None | Some("") | Some("file") => PreviewResource::LocalCandidate,
        Some("http" | "https" | "data") => PreviewResource::Browser,
        Some(_) => PreviewResource::Drop,
    }
}

/// Both answers packed for the wasm door: the scheme route in bits 0–2
/// (relative, web, mail, file, drive, defer, drop) and the preview answer
/// in bits 4–5 (local candidate, browser, drop). A number, not a string, so
/// a rendered link costs one crossing and no allocation on the way back.
pub fn scheme_route_code(dest: &str) -> u32 {
    let route = match scheme_route(dest) {
        SchemeRoute::Relative => 0,
        SchemeRoute::Web => 1,
        SchemeRoute::Mail => 2,
        SchemeRoute::File => 3,
        SchemeRoute::Drive => 4,
        SchemeRoute::Defer(_) => 5,
        SchemeRoute::Drop => 6,
    };
    let preview = match preview_resource(dest) {
        PreviewResource::LocalCandidate => 0,
        PreviewResource::Browser => 1,
        PreviewResource::Drop => 2,
    };
    route | (preview << 4)
}
