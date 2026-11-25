//! Which local file or attachment an authored destination names: the one
//! owner of the route (page, web, attachment, graph file, drive file …), the
//! attachment identity and the PDF reader identity. The media kind is not its
//! question: `src/media.ts` answers it.
//!
//! Pure: it reads only the destination text and where it was written, costs
//! O(destination bytes) and does no I/O, so its answer cannot go stale. The
//! graph root (the only input that can) is applied later by [`resolve_file`].
//!
//! Rules already settled: only a folder literally written `assets` (any case)
//! makes an attachment, and a folder name spelled with escapes is an ordinary
//! path; file names are decoded once; decoding comes before normalizing, so an
//! escaped backslash (`%5C`) is a separator; a drive path never names an
//! attachment. Where a product question is still open, this module keeps the
//! renderer's current answer and says so at that site ("Pending decision").

use crate::link_scheme::{anchor_route, scheme_route, AnchorRoute, SchemeRoute};
use crate::lsdoc::ast::Url;
// The strict decoder and the ECMAScript trim set are the outline's: one Rust
// answer each (pinned by `owner_holds_no_decoder_or_whitespace_set`).
use tine_outline::editor_text::js_blank;
pub(crate) use tine_outline::editor_text::js_trim;
use tine_outline::page_filename::decode_uri_component as strict_decode;

/// Where a destination was written. The route can depend on it (Markdown
/// links never open a PDF reader; embeds and Org links do).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    MdLink,
    MdEmbed,
    OrgLink,
    OrgBare,
    /// The `img` macro: its URL is the first whitespace-separated token of the
    /// parser's first argument (the parser splits arguments on commas only).
    ImgMacro,
    /// A raw HTML or Hiccup resource attribute (`src`, `href`, `poster`).
    HtmlResource,
    PageIcon,
}

impl Origin {
    pub fn parse(name: &str) -> Option<Origin> {
        Some(match name {
            "md-link" => Origin::MdLink,
            "md-embed" => Origin::MdEmbed,
            "org-link" => Origin::OrgLink,
            "org-bare" => Origin::OrgBare,
            "img-macro" => Origin::ImgMacro,
            "html-resource" => Origin::HtmlResource,
            "page-icon" => Origin::PageIcon,
            _ => return None,
        })
    }

    fn opens_pdf_reader(self) -> bool {
        matches!(self, Origin::MdEmbed | Origin::OrgLink | Origin::OrgBare)
    }
}

/// A parser link node's `url`, or the text of a macro argument, HTML
/// attribute or page icon value. A link's destination is the parser's `v`,
/// or for a `Complex` URL [`complex_destination`] of its fields; the link's
/// source text is never read.
#[derive(Debug, Clone, Copy)]
pub enum Destination<'a> {
    Link { url: &'a Url },
    Text(&'a str),
}

/// A name under the graph's `assets/` folder; empty means the folder itself.
/// Only [`local_target`] makes one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AssetName(String);

impl AssetName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutsideReason {
    Absolute,
    Escapes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    PageRef {
        name: String,
    },
    BlockRef {
        id: String,
    },
    Web,
    Mail,
    AppScheme {
        scheme: String,
    },
    Refused,
    /// Left to the browser or opener as authored: a fragment or relative link.
    Fragment {
        href: String,
    },
    Attachment {
        name: AssetName,
    },
    GraphFile {
        path: String,
    },
    OutsideFile {
        path: String,
        reason: OutsideReason,
    },
    Drive {
        path: String,
    },
    /// An explicit absolute `file:` URL, handed to the native `file:` gate as written.
    FileUrl {
        url: String,
    },
    /// A page icon path, resolved against the owning page's folder.
    PageRelative {
        path: String,
    },
    RefusedMalformed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pdf {
    /// Tine's reader with highlights, keyed on this name.
    AssetReader { name: AssetName },
    /// The outside-file reader; `fallback` is the asset reader used where the
    /// outside reader is unavailable (mobile, or no graph root).
    OutsideReader { fallback: Option<AssetName> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalTarget {
    route: Route,
    pdf: Option<Pdf>,
}

impl LocalTarget {
    pub fn route(&self) -> &Route {
        &self.route
    }
    pub fn pdf(&self) -> Option<&Pdf> {
        self.pdf.as_ref()
    }
    /// The attachment this destination keeps alive, if any.
    pub fn attachment(&self) -> Option<&AssetName> {
        match &self.route {
            Route::Attachment { name } => Some(name),
            _ => None,
        }
    }
    /// The JSON the TypeScript wrapper reads (`src/localTarget.ts`). Written by
    /// hand: derived serde code would push the browser parser module over its
    /// size ceiling. O(answer bytes).
    pub fn to_json(&self) -> String {
        // The kind with its closing quote, then the field's key when it has one.
        let (head, value) = match &self.route {
            Route::PageRef { name } => (r#"page_ref","name":"#, Some(name.as_str())),
            Route::BlockRef { id } => (r#"block_ref","id":"#, Some(id.as_str())),
            Route::Web => (r#"web""#, None),
            Route::Mail => (r#"mail""#, None),
            Route::AppScheme { scheme } => (r#"app_scheme","scheme":"#, Some(scheme.as_str())),
            Route::Refused => (r#"refused""#, None),
            Route::Fragment { href } => (r#"fragment","href":"#, Some(href.as_str())),
            Route::Attachment { name } => (r#"attachment","name":"#, Some(name.as_str())),
            Route::GraphFile { path } => (r#"graph_file","path":"#, Some(path.as_str())),
            Route::OutsideFile { path, .. } => (r#"outside_file","path":"#, Some(path.as_str())),
            Route::Drive { path } => (r#"drive","path":"#, Some(path.as_str())),
            Route::FileUrl { url } => (r#"file_url","url":"#, Some(url.as_str())),
            Route::PageRelative { path } => (r#"page_relative","path":"#, Some(path.as_str())),
            Route::RefusedMalformed => (r#"refused_malformed""#, None),
        };
        let mut out = String::from(r#"{"route":{"kind":""#);
        let quote =
            |out: &mut String, v: &str| out.push_str(&serde_json::to_string(v).unwrap_or_default());
        out.push_str(head);
        if let Some(value) = value {
            quote(&mut out, value);
        }
        if let Route::OutsideFile { reason, .. } = &self.route {
            out.push_str(if *reason == OutsideReason::Absolute {
                r#","reason":"absolute""#
            } else {
                r#","reason":"escapes""#
            });
        }
        out.push_str(r#"},"pdf":"#);
        match &self.pdf {
            None => out.push_str("null"),
            Some(Pdf::AssetReader { name }) => {
                out.push_str(r#"{"kind":"asset_reader","name":"#);
                quote(&mut out, name.as_str());
                out.push('}');
            }
            Some(Pdf::OutsideReader { fallback }) => {
                out.push_str(r#"{"kind":"outside_reader","fallback":"#);
                match fallback {
                    Some(name) => quote(&mut out, name.as_str()),
                    None => out.push_str("null"),
                }
                out.push('}');
            }
        }
        out.push('}');
        out
    }
    fn of(route: Route) -> LocalTarget {
        LocalTarget { route, pdf: None }
    }
}

/// Decide the route and identities of one destination. O(destination bytes).
pub fn local_target(dest: Destination<'_>, origin: Origin) -> LocalTarget {
    if origin == Origin::PageIcon {
        return match dest {
            Destination::Link {
                url: Url::Search { v } | Url::File { v } | Url::PageRef { v },
                ..
            } => page_icon(v),
            Destination::Text(text) => page_icon(text),
            _ => LocalTarget::of(Route::RefusedMalformed),
        };
    }
    let (text, search) = match dest {
        Destination::Link { url } => match url {
            Url::BlockRef { v } => return LocalTarget::of(Route::BlockRef { id: v.clone() }),
            // OG `show-link?`: a bare Org reference to an asset or media file
            // is an image, so a file rather than a page.
            Url::PageRef { v } if origin == Origin::OrgBare && org_bare_image(v) => {
                (v.clone(), false)
            }
            // Design: a bare Org drive link names the drive file, not a page.
            Url::PageRef { v }
                if origin == Origin::OrgBare && is_drive(&decode_once(js_trim(v))) =>
            {
                return path_target(v, SchemeRoute::Drive, origin);
            }
            Url::PageRef { v } => return LocalTarget::of(Route::PageRef { name: v.clone() }),
            Url::Search { v } => (v.clone(), true),
            Url::File { v } | Url::EmbedData { v } => (v.clone(), false),
            Url::Complex {
                protocol: Some(p),
                link: Some(l),
            } => (complex_destination(p, l), false),
            // The parser always fills both fields.
            Url::Complex { .. } => return LocalTarget::of(Route::RefusedMalformed),
        },
        Destination::Text(text) => (text.to_owned(), false),
    };
    if origin == Origin::HtmlResource {
        return html_resource(&text);
    }
    let decision = scheme_route(&text);
    // Pending decision 3 (dotless drive targets): like the renderer, a dotless
    // scheme-less search target, `C:/folder` included, is a page reference.
    if search
        && matches!(decision, SchemeRoute::Relative | SchemeRoute::Drive)
        && search_target_is_page(&text)
    {
        return LocalTarget::of(Route::PageRef { name: text });
    }
    match decision {
        SchemeRoute::Drop => LocalTarget::of(Route::Refused),
        SchemeRoute::Web => LocalTarget::of(Route::Web),
        SchemeRoute::Mail => LocalTarget::of(Route::Mail),
        SchemeRoute::Defer(scheme) => LocalTarget::of(Route::AppScheme { scheme }),
        _ => path_target(&text, decision, origin),
    }
}

/// The destination of a parser `Complex` URL, built from its fields as
/// Logseq does (`frontend/components/block.cljs`, `string-of-url`): the bare
/// `link` for the protocol `file` (exact case), otherwise
/// `protocol://link`. The parser drops what stood between `protocol:` and
/// `link`, so `C:x` and `C://x` (and `file://` and `file:////`) arrive alike
/// and are read alike, as in Logseq. O(link bytes).
pub fn complex_destination(protocol: &str, link: &str) -> String {
    if protocol == "file" {
        link.to_owned()
    } else {
        [protocol, "://", link].concat()
    }
}

/// Resolve a file route against the graph root bound when the user acted.
/// Parent steps above the root are kept; a drive root clamps them. O(path).
pub fn resolve_file(route: &Route, root: &str) -> Option<String> {
    let relative = match route {
        Route::Drive { path } => return Some(path.clone()),
        Route::OutsideFile {
            path,
            reason: OutsideReason::Absolute,
        } => return Some(path.clone()),
        Route::OutsideFile { path, .. } | Route::GraphFile { path } => path.clone(),
        Route::Attachment { name } if name.0.is_empty() => "assets".to_owned(),
        Route::Attachment { name } => format!("assets/{}", name.0),
        _ => return None,
    };
    let base = root.replace('\\', "/");
    let prefix = if base.starts_with("//") {
        "//"
    } else if base.starts_with('/') {
        "/"
    } else {
        ""
    };
    let mut segments: Vec<&str> = Vec::new();
    let joined = format!("{base}/{relative}");
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if segments.last().is_some_and(|last| !is_drive_root(last)) {
                    segments.pop();
                }
            }
            part => segments.push(part),
        }
    }
    Some(format!("{prefix}{}", segments.join("/")))
}

/// A scheme-less, file: or drive destination: attachment, graph file, drive
/// file or outside file, with its PDF identity.
fn path_target(dest: &str, decision: SchemeRoute, origin: Origin) -> LocalTarget {
    // Pending decision 2 (whitespace trimming): JavaScript's trim, as rendered today.
    let mut text = js_trim(dest);
    if matches!(decision, SchemeRoute::File) {
        text = text.get(5..).unwrap_or_default();
        if matches!(text.as_bytes().first(), Some(b'/' | b'\\')) {
            return file_url(dest.trim(), origin);
        }
    }
    if matches!(text.as_bytes().first(), None | Some(b'#' | b'*')) {
        return LocalTarget::of(Route::Fragment {
            href: dest.to_owned(),
        });
    }
    let decoded = decode_once(text).replace('\\', "/");
    let mut target = if is_drive(&decoded) {
        if decoded.as_bytes().get(2) != Some(&b'/') {
            // `C:docs` is relative to a drive's current folder: never opened.
            return LocalTarget::of(Route::RefusedMalformed);
        }
        LocalTarget::of(Route::Drive {
            path: normalize_drive(&decoded),
        })
    } else if let Some(name) = matches!(decision, SchemeRoute::Relative)
        .then(|| attachment_name(text))
        .flatten()
    {
        let pdf = is_pdf(&name.0).then(|| Pdf::AssetReader { name: name.clone() });
        let mut target = LocalTarget::of(Route::Attachment { name });
        target.pdf = pdf;
        target
    } else if decoded.starts_with('/') {
        LocalTarget::of(Route::OutsideFile {
            path: decoded.clone(),
            reason: OutsideReason::Absolute,
        })
    } else if !decoded.contains('.') {
        return LocalTarget::of(Route::Fragment {
            href: dest.to_owned(),
        });
    } else {
        LocalTarget::of(graph_relative(&decoded))
    };
    if target.pdf.is_none() && is_pdf(&decoded) {
        target.pdf = file_pdf(&target.route, &decoded, origin);
    }
    target
}

/// PDF identity of a file route. O(path bytes).
fn file_pdf(route: &Route, decoded: &str, origin: Origin) -> Option<Pdf> {
    let base = decoded
        .bytes()
        .rposition(|b| b == b'/')
        .map_or(0, |i| i + 1);
    let basename = || AssetName(decoded.get(base..).unwrap_or_default().to_owned());
    match route {
        // A bare `x.pdf` keeps Logseq's assets-relative reader.
        Route::GraphFile { .. } if !decoded.contains('/') => {
            Some(Pdf::AssetReader { name: basename() })
        }
        _ if !origin.opens_pdf_reader() => None,
        // Pending decision 4 (PDF fallback): where the outside reader is
        // unavailable, a graph-relative PDF opens the asset reader on its basename.
        Route::GraphFile { .. } => Some(Pdf::OutsideReader {
            fallback: Some(basename()),
        }),
        Route::OutsideFile { .. } | Route::Drive { .. } => {
            Some(Pdf::OutsideReader { fallback: None })
        }
        _ => None,
    }
}

/// The text after a leading `file://` (any case), if it has one.
fn after_file_slashes(url: &str) -> Option<&str> {
    url.get(..7)
        .filter(|p| p.eq_ignore_ascii_case("file://"))
        .and(url.get(7..))
}

fn file_url(url: &str, origin: Origin) -> LocalTarget {
    let mut target = LocalTarget::of(Route::FileUrl {
        url: url.to_owned(),
    });
    // A `file://` URL names a PDF when its path ends in `.pdf` and it carries
    // no query or fragment (no URL parser: it would double the wasm module).
    let pdf = after_file_slashes(url).is_some()
        && query_start(url) == url.len()
        && !url.contains("%00")
        && is_pdf(&percent_decode(url));
    if origin.opens_pdf_reader() && pdf {
        target.pdf = Some(Pdf::OutsideReader { fallback: None });
    }
    target
}

/// The name after the first folder literally written `assets` (any case). A
/// separator may be `/`, `\` or an escaped backslash; the remainder is decoded
/// once and normalized. None when no such folder occurs. O(bytes).
fn attachment_name(raw: &str) -> Option<AssetName> {
    let b = raw.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i <= b.len() {
        let sep = match b.get(i) {
            None => Some(0),
            Some(b'/') | Some(b'\\') => Some(1),
            Some(b'%')
                if b.get(i + 1) == Some(&b'5') && matches!(b.get(i + 2), Some(b'C' | b'c')) =>
            {
                Some(3)
            }
            _ => None,
        };
        let Some(len) = sep else {
            i += 1;
            continue;
        };
        if raw
            .get(start..i)
            .is_some_and(|folder| folder.eq_ignore_ascii_case("assets"))
        {
            let rest = decode_once(raw.get(i + len..).unwrap_or_default()).replace('\\', "/");
            let mut name = Vec::new();
            push_segments(&mut name, &rest, false);
            return Some(AssetName(name.join("/")));
        }
        i += len.max(1);
        start = i;
    }
    None
}

/// Push `path`'s `/`-separated segments, without empty and `.` ones. With
/// `parents`, a `..` removes the segment before it; one with nothing before
/// it to remove is kept when `segments` started empty and dropped otherwise
/// (a drive root clamps it). O(path bytes).
fn push_segments<'a>(segments: &mut Vec<&'a str>, path: &'a str, parents: bool) {
    let floor = segments.len();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." if parents => {
                if segments.len() > floor && segments.last() != Some(&"..") {
                    segments.pop();
                } else if floor == 0 {
                    segments.push(part);
                }
            }
            part => segments.push(part),
        }
    }
}

fn graph_relative(path: &str) -> Route {
    let mut segments = Vec::new();
    push_segments(&mut segments, path, true);
    let path = segments.join("/");
    if segments.first() == Some(&"..") {
        Route::OutsideFile {
            path,
            reason: OutsideReason::Escapes,
        }
    } else {
        Route::GraphFile { path }
    }
}

fn normalize_drive(path: &str) -> String {
    let (drive, rest) = path.split_at_checked(2).unwrap_or((path, ""));
    let mut segments = vec![drive];
    push_segments(&mut segments, rest, true);
    let mut out = segments.join("/");
    if segments.len() == 1 {
        out.push('/');
    }
    out
}

/// Pending decision 5 (raw HTML and Hiccup resources): today only an absolute
/// local `<img src>` is read; everything else stays with the browser.
fn html_resource(src: &str) -> LocalTarget {
    let file = after_file_slashes(src);
    let local = src.starts_with('/') && !src.starts_with("//")
        || file.is_some()
        || src.starts_with("\\\\")
        || is_drive(src) && matches!(src.as_bytes().get(2), Some(b'/' | b'\\'));
    if !local {
        return LocalTarget::of(match anchor_route(src) {
            AnchorRoute::Web => Route::Web,
            AnchorRoute::Drop | AnchorRoute::Defer | AnchorRoute::Mail => Route::Refused,
            _ => Route::Fragment {
                href: src.to_owned(),
            },
        });
    }
    let path = file.unwrap_or(src);
    let path = strict_decode(path).unwrap_or_else(|| path.to_owned());
    // `/C:/x` names the drive path `C:/x`.
    let drive = path
        .strip_prefix('/')
        .filter(|rest| is_drive(rest) && matches!(rest.as_bytes().get(2), Some(b'/' | b'\\')));
    LocalTarget::of(match drive {
        Some(drive) => Route::Drive {
            path: drive.to_owned(),
        },
        None if is_drive(&path) => Route::Drive { path },
        None => Route::OutsideFile {
            path,
            reason: OutsideReason::Absolute,
        },
    })
}

/// Pending decision 6 (page icons): icons keep their page-relative rule; a
/// malformed escape or a `:`, `\`, `?`, `#` or leading `/` refuses the icon.
fn page_icon(value: &str) -> LocalTarget {
    let value = value.strip_prefix("file:").unwrap_or(value);
    match strict_decode(value) {
        Some(path)
            if !path.is_empty()
                && !path.starts_with('/')
                && !path
                    .bytes()
                    .any(|b| matches!(b, b':' | b'\\' | 0 | b'?' | b'#')) =>
        {
            LocalTarget::of(Route::PageRelative { path })
        }
        _ => LocalTarget::of(Route::RefusedMalformed),
    }
}

/// OG `show-link?` for a bare Org reference: an `assets` prefix or a media
/// suffix makes it an image, except pdf/mp4/ogg/webm. O(bytes).
fn org_bare_image(target: &str) -> bool {
    let lead = target
        .bytes()
        .take_while(|b| matches!(b, b'.' | b'/'))
        .count();
    let local = target
        .as_bytes()
        .get(lead..)
        .is_some_and(|rest| rest.starts_with(b"assets"));
    // The extension before any `?` or `#` (OG tests the text before `#` and
    // before `?`; both end at the first of them).
    let ext = ext_of(target);
    let has = |list: &[&str]| list.iter().any(|known| known.eq_ignore_ascii_case(ext));
    (local || has(OG_ORG_MEDIA)) && !has(&["pdf", "mp4", "ogg", "webm"])
}

const OG_ORG_MEDIA: &[&str] = &[
    "gif", "svg", "jpeg", "ico", "png", "jpg", "bmp", "webp", "mp3", "ogg", "mpeg", "wav", "m4a",
    "flac", "wma", "aac", "mp4", "webm", "mov", "flv", "avi", "mkv",
];
/// The extension, ignoring a `?`/`#` tail. O(bytes).
fn ext_of(name: &str) -> &str {
    extension(name.get(..query_start(name)).unwrap_or_default())
}

/// Where a URL's query or fragment starts (the length when it has neither).
fn query_start(url: &str) -> usize {
    url.bytes()
        .position(|b| b == b'?' || b == b'#')
        .unwrap_or(url.len())
}

fn extension(name: &str) -> &str {
    name.rfind('.')
        .and_then(|i| name.get(i + 1..))
        .unwrap_or_default()
}

fn is_pdf(name: &str) -> bool {
    let b = name.as_bytes();
    b.split_last_chunk::<4>()
        .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case(b".pdf"))
}

fn is_drive(path: &str) -> bool {
    let b = path.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

fn is_drive_root(segment: &str) -> bool {
    segment.len() == 2 && is_drive(segment)
}

/// OG `search-link-cp`: a dotless search target is a page, unless blank, an
/// anchor (`#x`, `*x`) or a block reference. O(bytes).
fn search_target_is_page(v: &str) -> bool {
    // Byte tests: `#`, `*` and `.` are ASCII.
    let b = v.as_bytes();
    if js_blank(v)
        || b.first() == Some(&b'#')
        || (b.first() == Some(&b'*') && b.last() != Some(&b'*'))
        || b.contains(&b'.')
    {
        return false;
    }
    !is_block_ref(v)
}

fn is_block_ref(v: &str) -> bool {
    // `((` + a hyphenated 8-4-4-4-12 hex id + `))`.
    let [b'(', b'(', id @ .., b')', b')'] = v.as_bytes() else {
        return false;
    };
    id.len() == 36
        && id.iter().enumerate().all(|(i, &c)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

/// The one decoder of file names. Pending decision 1 (bad escapes): an invalid
/// escape stays literal, as the Rust PDF identity decodes today.
fn decode_once(s: &str) -> String {
    percent_decode(s)
}

/// Resolve a graph asset relative to `assets/`, normalizing separators and case
/// of the directory name. None when no asset directory occurs. O(URL bytes).
pub fn asset_rel_path(url: &str) -> Option<String> {
    let normalized = url.replace('\\', "/");
    let i = normalized.to_ascii_lowercase().find("assets/")?;
    normalized.get(i + "assets/".len()..).map(str::to_owned)
}

/// PDF identity is its path under `assets/` (case-insensitive directory name),
/// or the basename for paths outside that directory. Decode URL escapes,
/// normalize separators and omit current-directory components. O(path bytes).
pub fn pdf_asset_file(path: &str) -> String {
    let path = percent_decode(path);
    if let Some(rel) = asset_rel_path(&path).filter(|rel| !rel.is_empty()) {
        rel.split('/')
            .filter(|part| *part != ".")
            .collect::<Vec<_>>()
            .join("/")
    } else {
        path.rsplit(['/', '\\'])
            .next()
            .filter(|part| !part.is_empty())
            .unwrap_or(&path)
            .to_owned()
    }
}

/// Generate a PDF pointer that resolves back to the literal asset name. O(path bytes).
pub(crate) fn pdf_asset_pointer(filename: &str) -> String {
    format!("../assets/{}", filename.replace('%', "%25"))
}

/// Decode `%XX` percent-escapes (UTF-8 aware, like JS `decodeURIComponent`). An
/// invalid or truncated escape is left literal rather than dropped.
pub fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        if let (b'%', Some(&[h, l])) = (c, b.get(i + 1..i + 3)) {
            if let (Some(h), Some(l)) = (hex_nibble(h), hex_nibble(l)) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
#[path = "local_target_tests.rs"]
mod table;
