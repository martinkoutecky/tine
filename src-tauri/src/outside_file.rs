//! May a `file:` link target be opened or read now (tine#985)?
//!
//! The one answer to that question for every desktop route: note links
//! (`open_external`), the outside-PDF reader (`read_external_pdf`), rendered
//! graph-relative links (`open_graph_file`) and Settings' manifest buttons.
//! Refusal scenario (in scope: a shared or imported graph is untrusted
//! content): a link in a note must not hand an outside-graph file to the OS, or
//! have its bytes read, before the user agrees in a prompt naming it. Tine does
//! not defend against a process that can write the filesystem, so a swap during
//! the walk is out of scope (the storage trust model).
//!
//! `containment` either proves that a path stays inside the graph folder,
//! probing only entries whose parent is already proven inside, or answers
//! `Unproven` (outside, or not provable without probing outside). It never
//! refuses: an unproven target asks, and Open runs the route's own operation.
//! Every route then operates on the path that was admitted, normalized as the
//! platform resolves it, never on one re-derived from the link text.

#![cfg_attr(not(desktop), allow(dead_code))]

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// The error a `settings` caller gets for a target outside the graph. The
/// frontend's settings wrapper shows it as "Couldn't open the link".
pub(crate) const OUTSIDE_FILE: &str = "outside-file";
/// Wire prefix of an Ask answer: `file-consent:` and a JSON object. It is an
/// answer, not a failure; the frontend decodes it before failure reporting.
pub(crate) const FILE_CONSENT: &str = "file-consent:";

/// The symlink hops the platform itself follows in one path resolution, so
/// every chain the OS can resolve inside the graph is proven and opens with no
/// prompt. Linux and Android: `MAXSYMLINKS` (40, include/linux/namei.h). macOS
/// and iOS: `MAXSYMLINKS` (32, sys/param.h). Windows: 63 reparse points per
/// path (Microsoft, "Symbolic link effects on file system functions").
#[cfg(any(target_os = "linux", target_os = "android"))]
const POSIX_MAX_HOPS: usize = 40;
#[cfg(any(target_os = "macos", target_os = "ios"))]
const POSIX_MAX_HOPS: usize = 32;
const WINDOWS_MAX_HOPS: usize = 63;

/// Why the call is made. Absent means `Settings`, the strictest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Purpose {
    /// A note link's first call.
    Link,
    /// The call after the user said Open.
    LinkAgreed,
    /// Open, with "allow for this graph" ticked.
    LinkAgreedRemember,
    /// Settings' manifest buttons: never asks, never uses the grant.
    Settings,
}

impl Purpose {
    pub(crate) fn parse(purpose: Option<&str>) -> Result<Self, String> {
        match purpose {
            None | Some("settings") => Ok(Self::Settings),
            Some("link") => Ok(Self::Link),
            Some("link-agreed") => Ok(Self::LinkAgreed),
            Some("link-agreed-remember") => Ok(Self::LinkAgreedRemember),
            Some(other) => Err(format!("unknown file link purpose {other}")),
        }
    }
}

/// The caller's binding, already checked against its lease.
pub(crate) enum Binding<'a> {
    /// A graph window: the graph folder and this binding's grant.
    Graph {
        root_key: &'a Path,
        granted: &'a AtomicBool,
    },
    /// Quick Capture: no graph folder of its own, so nothing is proven inside
    /// and nothing is remembered. Its current show was checked by the caller.
    Capture,
}

/// What the route does next.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Admit {
    /// Run the route's operation on exactly this path.
    Go(PathBuf),
    /// Show the prompt; nothing was opened or read.
    Ask { display: String, lossy: bool },
}

impl Admit {
    /// `Go`'s path, or the Ask as its wire error.
    pub(crate) fn into_path(self) -> Result<PathBuf, String> {
        match self {
            Self::Go(path) => Ok(path),
            Self::Ask { display, lossy } => Err(format!(
                "{FILE_CONSENT}{}",
                serde_json::json!({ "display": display, "lossy": lossy })
            )),
        }
    }
}

/// The one decision. Cost: `containment`'s in-graph probes; no probe at all
/// for Quick Capture or a path lexically outside the graph.
pub(crate) fn admit(
    binding: &Binding<'_>,
    path: &Path,
    purpose: Purpose,
    probe: &mut impl Probe,
) -> Result<Admit, String> {
    let (admitted, display) = match binding {
        Binding::Capture => (path.to_path_buf(), path.as_os_str().to_owned()),
        Binding::Graph { root_key, .. } => match containment(SEMANTICS, root_key, path, probe) {
            Containment::Inside(admitted) | Containment::Missing(admitted) => {
                return Ok(Admit::Go(admitted))
            }
            Containment::Unproven { admitted, display } => (admitted, display),
        },
    };
    let granted = match binding {
        Binding::Graph { granted, .. } => Some(*granted),
        Binding::Capture => None,
    };
    match purpose {
        Purpose::Settings => Err(OUTSIDE_FILE.into()),
        Purpose::Link if granted.is_some_and(|bit| bit.load(Ordering::Acquire)) => {
            Ok(Admit::Go(admitted))
        }
        Purpose::Link => {
            let (display, lossy) = readable(&display);
            Ok(Admit::Ask { display, lossy })
        }
        Purpose::LinkAgreed => Ok(Admit::Go(admitted)),
        Purpose::LinkAgreedRemember => {
            // A record of the user's answer about this graph binding, whatever
            // the open then does. Capture never remembers.
            if let Some(bit) = granted {
                bit.store(true, Ordering::Release);
            }
            Ok(Admit::Go(admitted))
        }
    }
}

/// A display of `name` for the prompt: lossy UTF-8 with control characters
/// replaced, and whether anything was replaced. O(length).
fn readable(name: &OsStr) -> (String, bool) {
    let text = name.to_string_lossy();
    let mut lossy = name.to_str().is_none();
    let display = text
        .chars()
        .map(|c| {
            if c.is_control() {
                lossy = true;
                char::REPLACEMENT_CHARACTER
            } else {
                c
            }
        })
        .collect();
    (display, lossy)
}

/// One filesystem entry, seen without following it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Dir,
    File,
    Link,
}

/// The two probes the walker makes. Both look at the entry itself: `kind` is an
/// `lstat`, `read_link` reads the link's payload, never its target.
pub(crate) trait Probe {
    fn kind(&mut self, path: &Path) -> std::io::Result<Kind>;
    fn read_link(&mut self, path: &Path) -> std::io::Result<PathBuf>;
}

/// The real filesystem.
pub(crate) struct Fs;

impl Probe for Fs {
    fn kind(&mut self, path: &Path) -> std::io::Result<Kind> {
        let meta = std::fs::symlink_metadata(path)?;
        // On Windows `is_symlink` is a name-surrogate reparse point (symlink or
        // junction). Other reparse points (cloud placeholders, dedup) do not
        // redirect the name and are plain entries; a surrogate that
        // `read_link` cannot read is Unproven.
        Ok(if meta.file_type().is_symlink() {
            Kind::Link
        } else if meta.is_dir() {
            Kind::Dir
        } else {
            Kind::File
        })
    }
    fn read_link(&mut self, path: &Path) -> std::io::Result<PathBuf> {
        std::fs::read_link(path)
    }
}

/// Path semantics: how the platform resolves `..` and link payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Semantics {
    /// `..` is physical, resolved component by component by the kernel.
    #[cfg(unix)]
    Posix,
    /// `..` is removed lexically before any lookup (`GetFullPathNameW`).
    /// Available on every host so its rules are tested everywhere.
    #[cfg_attr(not(any(windows, test)), allow(dead_code))]
    Windows,
}

impl Semantics {
    fn max_hops(self) -> usize {
        match self {
            #[cfg(unix)]
            Self::Posix => POSIX_MAX_HOPS,
            Self::Windows => WINDOWS_MAX_HOPS,
        }
    }
    fn separator(self) -> &'static str {
        match self {
            #[cfg(unix)]
            Self::Posix => "/",
            Self::Windows => "\\",
        }
    }
}

#[cfg(unix)]
pub(crate) const SEMANTICS: Semantics = Semantics::Posix;
#[cfg(windows)]
pub(crate) const SEMANTICS: Semantics = Semantics::Windows;

/// The answer to "is this path inside the graph folder".
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Containment {
    /// Proven inside; `.0` is the admitted path.
    Inside(PathBuf),
    /// Proven not to exist (or a non-directory with components after it),
    /// without leaving the graph; the route's operation reports it.
    Missing(PathBuf),
    /// Outside, or not provable without probing outside. `display` names the
    /// file Open acts on as far as the walk knows it.
    Unproven {
        admitted: PathBuf,
        display: OsString,
    },
}

/// A parsed absolute path: a root (`/`, `C:\`, `\\srv\share\`) and its names.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Loc {
    root: OsString,
    parts: Vec<OsString>,
}

impl Loc {
    fn path(&self, sem: Semantics) -> PathBuf {
        let sep = sem.separator();
        let mut out = self.root.clone();
        for (index, part) in self.parts.iter().enumerate() {
            if index > 0 {
                out.push(sep);
            }
            out.push(part);
        }
        PathBuf::from(out)
    }
}

/// Parse `path` as the platform will resolve it, or `None` when it is not a
/// plain absolute path this walker can reason about (it then asks).
fn parse(sem: Semantics, path: &OsStr) -> Option<Loc> {
    match sem {
        #[cfg(unix)]
        Semantics::Posix => posix_parse(path),
        Semantics::Windows => windows_parse(path.to_str()?),
    }
}

#[cfg(unix)]
fn posix_parse(path: &OsStr) -> Option<Loc> {
    use std::path::Component;
    let mut components = Path::new(path).components();
    if components.next() != Some(Component::RootDir) {
        return None;
    }
    let mut parts = Vec::new();
    for component in components {
        match component {
            Component::Normal(name) => parts.push(name.to_owned()),
            // Kept: `..` is resolved physically, by the walk.
            Component::ParentDir => parts.push("..".into()),
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(Loc {
        root: "/".into(),
        parts,
    })
}

fn is_drive(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// Windows: the prefix becomes the root (drive letter upper-cased, verbatim
/// `\\?\` and NT `\??\` forms folded to their plain form), then `.` and `..`
/// are removed lexically, as `GetFullPathNameW` does before any lookup, with
/// `..` clamped at the root. A name ending in a dot or a space is trimmed by
/// Win32 in some positions and not others, so it is not reasoned about.
/// Device paths, volume GUID paths and drive-relative `D:x` are `None`.
fn windows_parse(path: &str) -> Option<Loc> {
    let path = path.replace('/', "\\");
    let (verbatim, rest) = match path
        .strip_prefix("\\\\?\\")
        .or_else(|| path.strip_prefix("\\??\\"))
    {
        Some(rest) => (true, rest.to_string()),
        None => (false, path.clone()),
    };
    let (root, tail) = if let Some(unc) = rest
        .strip_prefix("UNC\\")
        .filter(|_| verbatim)
        .or_else(|| rest.strip_prefix("\\\\").filter(|_| !verbatim))
    {
        let mut names = unc.splitn(3, '\\');
        let server = names.next().filter(|s| !s.is_empty())?;
        let share = names.next().filter(|s| !s.is_empty())?;
        if server == "." || server == "?" {
            return None;
        }
        (
            format!("\\\\{server}\\{share}\\"),
            names.next().unwrap_or("").to_string(),
        )
    } else if is_drive(rest.as_bytes()) && rest.as_bytes().get(2) == Some(&b'\\') {
        (
            format!("{}:\\", rest[..1].to_ascii_uppercase()),
            rest[3..].to_string(),
        )
    } else {
        return None;
    };
    let mut parts: Vec<OsString> = Vec::new();
    for name in tail.split('\\') {
        match name {
            "" | "." if !verbatim => {}
            ".." if !verbatim => {
                parts.pop();
            }
            "" | "." | ".." => return None,
            name if name.ends_with(['.', ' ']) => return None,
            name => parts.push(name.into()),
        }
    }
    Some(Loc {
        root: root.into(),
        parts,
    })
}

/// A link payload, classified by its platform meaning before the next probe.
enum Payload {
    /// Back to the prefix check.
    Absolute(Loc),
    /// Spliced into the rest of the walk at the link's parent.
    Relative(Vec<OsString>),
    /// Cannot be resolved without leaving the graph (`D:x`, an odd form).
    Unresolvable,
}

fn classify(sem: Semantics, payload: &Path, volume: &OsStr) -> Payload {
    match sem {
        #[cfg(unix)]
        Semantics::Posix => {
            use std::path::Component;
            if payload.has_root() {
                return parse(sem, payload.as_os_str())
                    .map_or(Payload::Unresolvable, Payload::Absolute);
            }
            Payload::Relative(
                payload
                    .components()
                    .filter_map(|component| match component {
                        Component::Normal(name) => Some(name.to_owned()),
                        Component::ParentDir => Some("..".into()),
                        _ => None,
                    })
                    .collect(),
            )
        }
        Semantics::Windows => {
            let Some(text) = payload.to_str() else {
                return Payload::Unresolvable;
            };
            let text = text.replace('/', "\\");
            let bytes = text.as_bytes();
            if text.starts_with("\\\\") || text.starts_with("\\??\\") {
                windows_parse(&text).map_or(Payload::Unresolvable, Payload::Absolute)
            } else if is_drive(bytes) {
                // `D:\T` is absolute; `D:T` depends on drive D's current folder.
                if bytes.get(2) == Some(&b'\\') {
                    windows_parse(&text).map_or(Payload::Unresolvable, Payload::Absolute)
                } else {
                    Payload::Unresolvable
                }
            } else if let Some(rooted) = text.strip_prefix('\\') {
                // The root of the volume that holds the link (taken from the
                // proven path, never from the process's current drive).
                let mut joined = volume.to_string_lossy().into_owned();
                joined.push_str(rooted);
                windows_parse(&joined).map_or(Payload::Unresolvable, Payload::Absolute)
            } else {
                // Relative: Win32 removes `x\..` inside the payload lexically;
                // leading `..` steps climb from the link's parent.
                let mut parts: Vec<OsString> = Vec::new();
                for name in text.split('\\') {
                    match name {
                        "" | "." => {}
                        ".." => {
                            if parts.last().is_some_and(|last| last != "..") {
                                parts.pop();
                            } else {
                                parts.push("..".into());
                            }
                        }
                        name if name.ends_with(['.', ' ']) => return Payload::Unresolvable,
                        name => parts.push(name.into()),
                    }
                }
                Payload::Relative(parts)
            }
        }
    }
}

/// Is `path` inside the graph folder `root_key`? Never probes outside it:
/// 1. the path's root and leading names must equal the graph folder's exactly
///    (another drive, host or a case alias is Unproven, zero probes);
/// 2. the rest is walked on a stack of physical components: an entry is
///    `lstat`ed only when its parent is proven inside, `..` pops (below the
///    root is Unproven), a link's payload is read and classified, and the hop
///    bound is the platform's own.
///
/// Cost: one `lstat` per component below the root plus one `read_link` per
/// hop, at most the platform's hop bound.
pub(crate) fn containment(
    sem: Semantics,
    root_key: &Path,
    path: &Path,
    probe: &mut impl Probe,
) -> Containment {
    let unproven = |display: OsString| Containment::Unproven {
        admitted: path.to_path_buf(),
        display,
    };
    let Some(key) = parse(sem, root_key.as_os_str()) else {
        return unproven(path.as_os_str().to_owned());
    };
    let Some(input) = parse(sem, path.as_os_str()) else {
        return unproven(path.as_os_str().to_owned());
    };
    // Windows: the admitted path is the lexically normalized one, which is what
    // the OS itself resolves; every route operates on it, so `alias\..` can
    // never be proven one way and opened another. POSIX resolves `..`
    // physically, as the walk does, so the path is admitted as it is.
    let admitted = match sem {
        #[cfg(unix)]
        Semantics::Posix => path.to_path_buf(),
        Semantics::Windows => input.path(sem),
    };
    let unproven = |display: OsString| Containment::Unproven {
        admitted: admitted.clone(),
        display,
    };
    let mut current = input;
    let mut hops = 0usize;
    'restart: loop {
        if current.root != key.root || !current.parts.starts_with(&key.parts) {
            return unproven(current.path(sem).into_os_string());
        }
        let mut stack = key.parts.clone();
        let mut rest: std::collections::VecDeque<OsString> =
            current.parts[key.parts.len()..].iter().cloned().collect();
        while let Some(name) = rest.pop_front() {
            if name == "." {
                continue;
            }
            if name == ".." {
                if stack.len() == key.parts.len() {
                    if key.parts.is_empty() {
                        continue; // `/..` is `/`
                    }
                    let mut parts = stack[..stack.len() - 1].to_vec();
                    parts.extend(rest);
                    let loc = Loc {
                        root: key.root.clone(),
                        parts,
                    };
                    return unproven(loc.path(sem).into_os_string());
                }
                stack.pop();
                continue;
            }
            stack.push(name);
            let here = Loc {
                root: key.root.clone(),
                parts: stack.clone(),
            }
            .path(sem);
            match probe.kind(&here) {
                Ok(Kind::Dir) => {}
                Ok(Kind::File) => {
                    if rest.iter().any(|name| name != ".") {
                        return Containment::Missing(admitted);
                    }
                }
                Ok(Kind::Link) => {
                    hops += 1;
                    if hops > sem.max_hops() {
                        return unproven(path.as_os_str().to_owned());
                    }
                    let Ok(payload) = probe.read_link(&here) else {
                        return unproven(path.as_os_str().to_owned());
                    };
                    stack.pop();
                    match classify(sem, &payload, &key.root) {
                        Payload::Absolute(mut target) => {
                            target.parts.extend(rest);
                            current = target;
                            continue 'restart;
                        }
                        Payload::Relative(parts) => {
                            for part in parts.into_iter().rev() {
                                rest.push_front(part);
                            }
                        }
                        Payload::Unresolvable => {
                            let mut display = payload.into_os_string();
                            for name in rest {
                                display.push(sem.separator());
                                display.push(name);
                            }
                            return unproven(display);
                        }
                    }
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound
                        || error.kind() == std::io::ErrorKind::NotADirectory =>
                {
                    return Containment::Missing(admitted);
                }
                Err(_) => return unproven(path.as_os_str().to_owned()),
            }
        }
        return Containment::Inside(admitted);
    }
}

/// The binding a `file:` call is admitted under. A graph window's call must
/// carry its current lease. Quick Capture has no slot (`state.rs`); its call
/// must carry the show it came from, and that show must still be current and
/// visible, so a prompt from show A can never open anything in show B, even
/// when the WebView missed `capture-shown`.
pub(crate) fn caller_slot(
    ctx: &crate::state::GraphContext<'_>,
    show: Option<u64>,
) -> Result<Option<std::sync::Arc<crate::state::GraphSlot>>, String> {
    if ctx.window.label() == "capture" {
        let visible = ctx.window.is_visible().unwrap_or(false);
        capture_show_admits(&ctx.state, show, visible)?;
        return Ok(None);
    }
    crate::state::slot_for_context(ctx).map(Some)
}

pub(crate) fn capture_show_admits(
    state: &crate::state::AppState,
    show: Option<u64>,
    visible: bool,
) -> Result<(), String> {
    match show {
        Some(show) if visible && state.capture_show_is_current(show) => Ok(()),
        _ => Err(crate::state::STALE_BINDING.into()),
    }
}

pub(crate) fn binding_of(slot: Option<&crate::state::GraphSlot>) -> Binding<'_> {
    match slot {
        Some(slot) => Binding::Graph {
            root_key: &slot.root_key,
            granted: &slot.outside_files_granted,
        },
        None => Binding::Capture,
    }
}

/// P1, a note link's local path: admit, then develop's operation on the
/// admitted path (it must exist; the OS opener receives it).
pub(crate) fn open_local(
    binding: &Binding<'_>,
    path: &Path,
    purpose: Purpose,
    probe: &mut impl Probe,
    open: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let path = admit(binding, path, purpose, probe)?.into_path()?;
    // Every OS opener accepts a path that is not there — `explorer.exe` even
    // opens an unrelated window for one — so without this check a stale link
    // reproduces the reported "clicking does nothing" from the other side.
    if !path.exists() {
        return Err(format!("{} could not be found", path.display()));
    }
    open(&path)
}

/// P3, a rendered graph-relative link: develop's lexical refusal, then admit
/// `root/clean`, then develop's canonical check on the admitted path, which
/// refuses a final target outside the graph (after consent) and opens one
/// inside, including a link that leaves the graph and comes back.
pub(crate) fn open_graph_relative(
    root_key: &Path,
    granted: &AtomicBool,
    relative: &str,
    purpose: Purpose,
    probe: &mut impl Probe,
    open: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let clean = crate::platform::graph_relative_path(relative)?;
    let binding = Binding::Graph { root_key, granted };
    let admitted = admit(&binding, &root_key.join(&clean), purpose, probe)?.into_path()?;
    open(&crate::platform::graph_relative_target(
        root_key, &admitted, &clean,
    )?)
}

#[cfg(test)]
#[path = "outside_file_tests.rs"]
mod tests;
