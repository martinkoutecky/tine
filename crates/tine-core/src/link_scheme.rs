//! Which scheme route an authored destination takes: the one rule, read by
//! the link-target owner (`local_target`) and, through wasm, by the
//! renderer's scheme policy (`src/linkSchemePolicy.ts`). The native handoff
//! gate (`src-tauri/src/platform.rs`) rechecks every OS handoff separately.
//!
//! Read on the authored text before any decoding (a scheme is never spelled
//! with escapes). One-letter prefixes name a drive. Only the first
//! [`ROUTE_PREFIX_BYTES`] bytes are read, so a caller may pass just that
//! prefix; a destination still undecided after them is refused. Pure, O(1)
//! per call (at most that many bytes), no allocation for the wasm entries,
//! no I/O.

/// How many leading bytes the rule reads. A destination not decided within
/// them (a scheme name, or leading spaces and control bytes, running past
/// them) is refused. 512 is twice the 255-byte limit on one file-name
/// component, so a scheme-less name with its leading whitespace is always
/// decided: before a `:` the rule stops at the first byte that cannot be in
/// a scheme (`/`, `?`, `#`, a space after the name, any non-ASCII byte).
pub const ROUTE_PREFIX_BYTES: usize = 512;

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
    /// A refused scheme, including one hidden by control bytes or spaces,
    /// and a destination undecided within [`ROUTE_PREFIX_BYTES`].
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

include!(concat!(env!("OUT_DIR"), "/refused_schemes.rs"));

/// The longest scheme name the rule tells apart (`javascript`); a longer
/// one is a custom scheme. A test checks every refused name fits.
const LONGEST_KNOWN: usize = 10;

/// Is `scheme` (lowercase) a member of the shared list's `refused` array?
/// The one answer: the native handoff gate and this rule both ask it.
/// O(list length); the list is compiled in from the shared file.
pub fn refused_scheme(scheme: &str) -> bool {
    refused(scheme.as_bytes())
}

fn refused(s: &[u8]) -> bool {
    REFUSED_SCHEMES.iter().any(|name| name.as_bytes() == s)
}

/// What the leading bytes spell, read the way a browser does: leading C0
/// controls and spaces dropped, tabs and line breaks ignored; `compact`
/// ignores every control and space.
enum Scan {
    NoScheme,
    Drive,
    /// A scheme name: its first [`LONGEST_KNOWN`] bytes lowercased, and its
    /// length.
    Name([u8; LONGEST_KNOWN], usize),
    /// Neither settled within [`ROUTE_PREFIX_BYTES`].
    Undecided,
}

impl Scan {
    /// The name when it is short enough to be a known one.
    fn known(&self) -> Option<&[u8]> {
        match self {
            Scan::Name(name, len) if *len <= LONGEST_KNOWN => Some(&name[..*len]),
            _ => None,
        }
    }
}

/// Scan at most [`ROUTE_PREFIX_BYTES`] bytes. A name longer than `longest`
/// settles as no scheme (the caller cannot tell such names apart). `collect`
/// receives the whole lowercased name. O(prefix bytes), no allocation
/// unless collecting.
fn scan(dest: &str, compact: bool, longest: usize, mut collect: Option<&mut String>) -> Scan {
    let mut name = [0u8; LONGEST_KNOWN];
    let mut len = 0;
    for &c in dest.as_bytes().iter().take(ROUTE_PREFIX_BYTES) {
        if c <= b' ' && (compact || len == 0 || matches!(c, b'\t' | b'\n' | b'\r')) {
            continue;
        }
        if c == b':' && len > 0 {
            return if len == 1 {
                Scan::Drive
            } else {
                Scan::Name(name, len)
            };
        }
        let valid = if len == 0 {
            c.is_ascii_alphabetic()
        } else {
            c.is_ascii_alphanumeric() || matches!(c, b'+' | b'.' | b'-')
        };
        if !valid {
            return Scan::NoScheme;
        }
        let lower = c.to_ascii_lowercase();
        if len < LONGEST_KNOWN {
            name[len] = lower;
        }
        if let Some(out) = collect.as_deref_mut() {
            out.push(lower as char);
        }
        len += 1;
        if len > longest {
            return Scan::NoScheme;
        }
    }
    if dest.len() >= ROUTE_PREFIX_BYTES {
        Scan::Undecided
    } else {
        Scan::NoScheme
    }
}

/// The lowercased URL scheme, `None` for no scheme, a drive letter or an
/// undecided destination. O(prefix bytes).
pub fn link_scheme(dest: &str) -> Option<String> {
    let mut name = String::new();
    match scan(dest, false, usize::MAX, Some(&mut name)) {
        Scan::Name(..) => Some(name),
        _ => None,
    }
}

/// The route, with the custom scheme's name only when `collect` is given.
fn route(dest: &str, collect: Option<&mut String>) -> AnchorRoute {
    let scan = scan(dest, false, usize::MAX, collect);
    match scan {
        Scan::Undecided => AnchorRoute::Drop,
        Scan::Drive => AnchorRoute::Drive,
        // A scheme hidden by embedded control bytes or spaces is still
        // refused; a compact name longer than any refused one cannot be.
        Scan::NoScheme => match self::scan(dest, true, LONGEST_KNOWN, None) {
            Scan::Undecided => AnchorRoute::Drop,
            hidden if hidden.known().is_some_and(refused) => AnchorRoute::Drop,
            _ => AnchorRoute::Relative,
        },
        Scan::Name(..) => match scan.known() {
            Some(s) if refused(s) => AnchorRoute::Drop,
            Some(b"http" | b"https") => AnchorRoute::Web,
            Some(b"mailto") => AnchorRoute::Mail,
            Some(b"file") if literal_file_prefix(dest) => AnchorRoute::File,
            _ => AnchorRoute::Defer,
        },
    }
}

/// The text, after JavaScript's leading whitespace, starts `file:` (any
/// case). Only asked once the scan read the scheme `file`, so every byte
/// before it is ASCII. O(prefix bytes).
fn literal_file_prefix(dest: &str) -> bool {
    let b = dest.as_bytes();
    let lead = b
        .iter()
        .take_while(|c| matches!(c, b'\t' | b'\n' | 0x0b | 0x0c | b'\r' | b' '))
        .count();
    b.get(lead..lead + 5)
        .is_some_and(|p| p.eq_ignore_ascii_case(b"file:"))
}

#[derive(Clone, Copy)]
enum AnchorRoute {
    Relative,
    Web,
    Mail,
    File,
    Drive,
    Defer,
    Drop,
}

/// The scheme route of an authored destination. O(prefix bytes).
pub fn scheme_route(dest: &str) -> SchemeRoute {
    let mut name = String::new();
    match route(dest, Some(&mut name)) {
        AnchorRoute::Relative => SchemeRoute::Relative,
        AnchorRoute::Web => SchemeRoute::Web,
        AnchorRoute::Mail => SchemeRoute::Mail,
        AnchorRoute::File => SchemeRoute::File,
        AnchorRoute::Drive => SchemeRoute::Drive,
        AnchorRoute::Defer => SchemeRoute::Defer(name),
        AnchorRoute::Drop => SchemeRoute::Drop,
    }
}

/// The anchor route as a number for the wasm door (relative, web, mail,
/// file, drive, defer, drop): no allocation either way. O(prefix bytes).
pub fn anchor_route_code(dest: &str) -> u32 {
    route(dest, None) as u32
}

/// The scheme route of a raw HTML resource attribute, every control byte
/// and space ignored. O(prefix bytes).
pub fn preview_resource(dest: &str) -> PreviewResource {
    let scan = scan(dest, true, usize::MAX, None);
    match scan {
        Scan::NoScheme | Scan::Drive => PreviewResource::LocalCandidate,
        Scan::Undecided => PreviewResource::Drop,
        Scan::Name(..) => match scan.known() {
            Some(b"file") => PreviewResource::LocalCandidate,
            Some(b"http" | b"https" | b"data") => PreviewResource::Browser,
            _ => PreviewResource::Drop,
        },
    }
}

/// The preview answer as a number for the wasm door (local candidate,
/// browser, drop). O(prefix bytes).
pub fn preview_resource_code(dest: &str) -> u32 {
    preview_resource(dest) as u32
}

#[cfg(test)]
#[path = "link_scheme_tests.rs"]
mod tests;
