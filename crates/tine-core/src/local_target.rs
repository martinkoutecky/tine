//! Which local file or attachment an authored destination names: the one
//! owner of the route (page, web, attachment, graph file, drive file …), the
//! attachment identity, the PDF reader identity and the media kind.
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

use crate::link_scheme::{scheme_route, SchemeRoute};
use crate::lsdoc::ast::Url;

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

/// A parser link node (its `url` and its `full` source text), or the text of
/// a macro argument, HTML attribute or page icon value. A link's destination
/// is the parser's `v`, or for a `Complex` URL the authored text located in
/// `full` (see [`authored`]); it is never rebuilt from the URL fields.
#[derive(Debug, Clone, Copy)]
pub enum Destination<'a> {
    Link { url: &'a Url, full: &'a str },
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Media {
    Image,
    Audio,
    Video,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalTarget {
    route: Route,
    pdf: Option<Pdf>,
    media: Option<Media>,
}

impl LocalTarget {
    pub fn route(&self) -> &Route {
        &self.route
    }
    pub fn pdf(&self) -> Option<&Pdf> {
        self.pdf.as_ref()
    }
    pub fn media(&self) -> Option<Media> {
        self.media
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
        let (kind, key, value) = match &self.route {
            Route::PageRef { name } => ("page_ref", "name", name.as_str()),
            Route::BlockRef { id } => ("block_ref", "id", id.as_str()),
            Route::Web => ("web", "", ""),
            Route::Mail => ("mail", "", ""),
            Route::AppScheme { scheme } => ("app_scheme", "scheme", scheme.as_str()),
            Route::Refused => ("refused", "", ""),
            Route::Fragment { href } => ("fragment", "href", href.as_str()),
            Route::Attachment { name } => ("attachment", "name", name.as_str()),
            Route::GraphFile { path } => ("graph_file", "path", path.as_str()),
            Route::OutsideFile { path, .. } => ("outside_file", "path", path.as_str()),
            Route::Drive { path } => ("drive", "path", path.as_str()),
            Route::FileUrl { url } => ("file_url", "url", url.as_str()),
            Route::PageRelative { path } => ("page_relative", "path", path.as_str()),
            Route::RefusedMalformed => ("refused_malformed", "", ""),
        };
        let mut out = String::from(r#"{"route":{"kind":""#);
        let quote =
            |out: &mut String, v: &str| out.push_str(&serde_json::to_string(v).unwrap_or_default());
        out.push_str(kind);
        out.push('"');
        if !key.is_empty() {
            out.push_str(",\"");
            out.push_str(key);
            out.push_str("\":");
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
        out.push_str(match self.media {
            None => r#","media":null}"#,
            Some(Media::Image) => r#","media":"image"}"#,
            Some(Media::Audio) => r#","media":"audio"}"#,
            Some(Media::Video) => r#","media":"video"}"#,
        });
        out
    }
    fn of(route: Route) -> LocalTarget {
        LocalTarget {
            route,
            pdf: None,
            media: None,
        }
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
    let mut image = false;
    let (text, search) = match dest {
        Destination::Link { url, full } => match url {
            Url::BlockRef { v } => return LocalTarget::of(Route::BlockRef { id: v.clone() }),
            // OG `show-link?`: a bare Org reference to an asset or media file is an image.
            Url::PageRef { v } if origin == Origin::OrgBare && org_bare_image(v) => {
                image = true;
                (v.clone(), false)
            }
            // Design: a bare Org drive link names the drive file, not a page.
            Url::PageRef { v }
                if origin == Origin::OrgBare
                    && is_drive(&decode_once(js_trim(v)).replace('\\', "/")) =>
            {
                return path_target(v, SchemeRoute::Drive, origin);
            }
            Url::PageRef { v } => return LocalTarget::of(Route::PageRef { name: v.clone() }),
            Url::Search { v } => (v.clone(), true),
            Url::File { v } | Url::EmbedData { v } => (v.clone(), false),
            Url::Complex {
                protocol: Some(p),
                link: Some(l),
            } => match authored(full, p, l, origin) {
                Some(text) => (text.to_owned(), false),
                None => return LocalTarget::of(Route::RefusedMalformed),
            },
            // The parser always fills both fields; nothing authored to read.
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
    let mut target = match decision {
        SchemeRoute::Drop => LocalTarget::of(Route::Refused),
        SchemeRoute::Web => LocalTarget::of(Route::Web),
        SchemeRoute::Mail => LocalTarget::of(Route::Mail),
        SchemeRoute::Defer(scheme) => LocalTarget::of(Route::AppScheme { scheme }),
        _ => path_target(&text, decision, origin),
    };
    if target.route == Route::Web {
        target.media = media_kind(&text[..query_start(&text)]);
    }
    if image
        && matches!(
            target.route,
            Route::Web | Route::Attachment { .. } | Route::GraphFile { .. }
        )
    {
        target.media = Some(Media::Image);
    }
    target
}

/// The destination as written, located in the parser's `full` link text. The
/// parser's `Complex` fields drop what stood between `protocol:` and `link`
/// (no slashes, `//`, or a second `//`) and Markdown escapes, so `[[C:x][y]]`
/// and `[[C://x][y]]` share their fields; only `full` tells them apart. An
/// autolink's `full` is its destination. Otherwise the destination is where
/// `protocol:`, then 4, 2 or no slashes, then `link` (a `\` before ASCII
/// punctuation skipped) is followed by the end of a destination: the last such
/// place in Markdown (the label comes first), the first in Org. None when
/// `full` holds none. O(full bytes) per `protocol:` occurrence tried.
fn authored<'a>(full: &'a str, protocol: &str, link: &str, origin: Origin) -> Option<&'a str> {
    let markdown = matches!(origin, Origin::MdLink | Origin::MdEmbed);
    let head = format!("{protocol}:");
    if full.starts_with(&head) {
        return Some(full);
    }
    let mut found = None;
    for (at, _) in full.match_indices(&head) {
        let start = at + head.len();
        let end = [4, 2, 0].into_iter().find_map(|slashes| {
            let tail = full.get(start + slashes..)?;
            if full[start..start + slashes].bytes().any(|b| b != b'/') {
                return None;
            }
            let end = start + slashes + escaped_prefix(tail, link)?;
            matches!(
                full.as_bytes().get(end),
                None | Some(b')' | b']' | b'>' | b' ' | b'\t' | b'\n' | b'\r')
            )
            .then_some(end)
        });
        if let Some(end) = end {
            found = Some(&full[at..end]);
            if !markdown {
                break;
            }
        }
    }
    found
}

/// How many bytes of `raw` spell `text`, where `raw` may escape ASCII
/// punctuation with `\`. None when `raw` does not begin with it. O(bytes).
fn escaped_prefix(raw: &str, text: &str) -> Option<usize> {
    let (raw, text) = (raw.as_bytes(), text.as_bytes());
    let (mut i, mut j) = (0, 0);
    while j < text.len() {
        if raw.get(i) == Some(&text[j]) {
            j += 1;
        } else if !(raw.get(i) == Some(&b'\\')
            && text[j].is_ascii_punctuation()
            && raw.get(i + 1) == Some(&text[j]))
        {
            return None;
        }
        i += 1;
    }
    Some(i)
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
/// file or outside file, with its PDF and media identities.
fn path_target(dest: &str, decision: SchemeRoute, origin: Origin) -> LocalTarget {
    // Pending decision 2 (whitespace trimming): JavaScript's trim, as rendered today.
    let mut text = js_trim(dest);
    if decision == SchemeRoute::File {
        text = &text[5..];
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
        if !decoded[2..].starts_with('/') {
            // `C:docs` is relative to a drive's current folder: never opened.
            return LocalTarget::of(Route::RefusedMalformed);
        }
        LocalTarget::of(Route::Drive {
            path: normalize_drive(&decoded),
        })
    } else if let Some(name) = (decision == SchemeRoute::Relative)
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
    if target.pdf.is_none() {
        target.media = media_kind(&decoded);
    }
    target
}

/// PDF identity of a file route. O(path bytes).
fn file_pdf(route: &Route, decoded: &str, origin: Origin) -> Option<Pdf> {
    let basename = || AssetName(decoded.rsplit('/').next().unwrap_or(decoded).to_owned());
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

fn file_url(url: &str, origin: Origin) -> LocalTarget {
    let mut target = LocalTarget::of(Route::FileUrl {
        url: url.to_owned(),
    });
    // A `file://` URL names a PDF when its path ends in `.pdf` and it carries
    // no query or fragment (no URL parser: it would double the wasm module).
    let pdf = url
        .get(..7)
        .is_some_and(|p| p.eq_ignore_ascii_case("file://"))
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
        if raw[start..i].eq_ignore_ascii_case("assets") {
            let rest = decode_once(&raw[i + len..]).replace('\\', "/");
            let name: Vec<&str> = rest
                .split('/')
                .filter(|p| !p.is_empty() && *p != ".")
                .collect();
            return Some(AssetName(name.join("/")));
        }
        i += len.max(1);
        start = i;
    }
    None
}

fn graph_relative(path: &str) -> Route {
    let mut segments: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." if segments.last().is_some_and(|last| *last != "..") => {
                segments.pop();
            }
            part => segments.push(part),
        }
    }
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
    let mut segments: Vec<&str> = vec![&path[..2]];
    for part in path[2..].split('/') {
        match part {
            "" | "." => {}
            ".." if segments.len() > 1 => {
                segments.pop();
            }
            ".." => {}
            part => segments.push(part),
        }
    }
    if segments.len() == 1 {
        return format!("{}/", segments[0]);
    }
    segments.join("/")
}

/// Pending decision 5 (raw HTML and Hiccup resources): today only an absolute
/// local `<img src>` is read; everything else stays with the browser.
fn html_resource(src: &str) -> LocalTarget {
    let local = src.starts_with('/') && !src.starts_with("//")
        || src
            .get(..7)
            .is_some_and(|p| p.eq_ignore_ascii_case("file://"))
        || src.starts_with("\\\\")
        || is_drive(src) && matches!(src.as_bytes().get(2), Some(b'/' | b'\\'));
    if !local {
        return LocalTarget::of(match scheme_route(src) {
            SchemeRoute::Web => Route::Web,
            SchemeRoute::Drop | SchemeRoute::Defer(_) | SchemeRoute::Mail => Route::Refused,
            _ => Route::Fragment {
                href: src.to_owned(),
            },
        });
    }
    let path = if src
        .get(..7)
        .is_some_and(|p| p.eq_ignore_ascii_case("file://"))
    {
        &src[7..]
    } else {
        src
    };
    let mut path = strict_decode(path).unwrap_or_else(|| path.to_owned());
    if path.starts_with('/')
        && is_drive(&path[1..])
        && matches!(path.as_bytes().get(3), Some(b'/' | b'\\'))
    {
        path.remove(0);
    }
    let mut target = LocalTarget::of(if is_drive(&path) {
        Route::Drive { path }
    } else {
        Route::OutsideFile {
            path,
            reason: OutsideReason::Absolute,
        }
    });
    target.media = Some(Media::Image);
    target
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
    let local = target[lead..].starts_with("assets");
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
/// The one media extension inventory (OG's media sets, so a clone graph stays
/// compatible): the owner classifies with it and `src/media.ts` reads it
/// through [`media_inventory`].
const MEDIA_EXTS: &[(&str, Media)] = &[
    ("png", Media::Image),
    ("jpg", Media::Image),
    ("jpeg", Media::Image),
    ("gif", Media::Image),
    ("webp", Media::Image),
    ("svg", Media::Image),
    ("bmp", Media::Image),
    ("avif", Media::Image),
    ("mp4", Media::Video),
    ("webm", Media::Video),
    ("mov", Media::Video),
    ("flv", Media::Video),
    ("avi", Media::Video),
    ("mkv", Media::Video),
    ("m4v", Media::Video),
    ("ogv", Media::Video),
    ("mp3", Media::Audio),
    ("ogg", Media::Audio),
    ("oga", Media::Audio),
    ("wav", Media::Audio),
    ("m4a", Media::Audio),
    ("flac", Media::Audio),
    ("wma", Media::Audio),
    ("aac", Media::Audio),
    ("opus", Media::Audio),
    ("mpeg", Media::Audio),
];

/// The one media inventory for every other reader (insertion, page icons,
/// playback in `src/media.ts`), as `ext:kind` pairs joined by commas. Read
/// once per session. O(inventory).
pub fn media_inventory() -> String {
    let mut out = String::new();
    for (ext, media) in MEDIA_EXTS {
        if !out.is_empty() {
            out.push(',');
        }
        out.push_str(ext);
        out.push_str(match media {
            Media::Image => ":image",
            Media::Audio => ":audio",
            Media::Video => ":video",
        });
    }
    out
}

/// The extension, ignoring a `?`/`#` tail. O(bytes).
fn ext_of(name: &str) -> &str {
    extension(&name[..query_start(name)])
}

/// Where a URL's query or fragment starts (the length when it has neither).
fn query_start(url: &str) -> usize {
    url.bytes()
        .position(|b| b == b'?' || b == b'#')
        .unwrap_or(url.len())
}

/// Media kind from the extension of a decoded name (a `?` or `#` in it is
/// part of the file name) or of a URL with its query and fragment removed.
fn media_kind(name: &str) -> Option<Media> {
    let ext = extension(name);
    MEDIA_EXTS
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(ext))
        .map(|(_, media)| *media)
}

fn extension(name: &str) -> &str {
    name.rfind('.').map_or("", |i| &name[i + 1..])
}

fn is_pdf(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() >= 4 && b[b.len() - 4..].eq_ignore_ascii_case(b".pdf")
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
    let first = v.chars().next();
    if js_trim(v).is_empty()
        || first == Some('#')
        || (first == Some('*') && !v.ends_with('*'))
        || v.contains('.')
    {
        return false;
    }
    !is_block_ref(v)
}

fn is_block_ref(v: &str) -> bool {
    // `((` + a hyphenated 8-4-4-4-12 hex id + `))`.
    let Some(id) = v.strip_prefix("((").and_then(|v| v.strip_suffix("))")) else {
        return false;
    };
    id.len() == 36
        && id.bytes().enumerate().all(|(i, c)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

/// JavaScript `String.prototype.trim`. O(bytes).
pub(crate) fn js_trim(s: &str) -> &str {
    let js_space = |c: char| {
        matches!(
            c,
            '\t' | '\n'
                | '\u{b}'
                | '\u{c}'
                | '\r'
                | ' '
                | '\u{a0}'
                | '\u{1680}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
                | '\u{2000}'..='\u{200a}'
        )
    };
    s.trim_matches(js_space)
}

/// The one decoder of file names. Pending decision 1 (bad escapes): an invalid
/// escape stays literal, as the Rust PDF identity decodes today.
fn decode_once(s: &str) -> String {
    percent_decode(s)
}

/// JavaScript `decodeURIComponent`: None on any invalid escape or UTF-8.
fn strict_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = hex_nibble(*b.get(i + 1)?)?;
            let l = hex_nibble(*b.get(i + 2)?)?;
            out.push((h << 4) | l);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Resolve a graph asset relative to `assets/`, normalizing separators and case
/// of the directory name. None when no asset directory occurs. O(URL bytes).
pub fn asset_rel_path(url: &str) -> Option<String> {
    let normalized = url.replace('\\', "/");
    let i = normalized.to_ascii_lowercase().find("assets/")?;
    Some(normalized[i + "assets/".len()..].to_owned())
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
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex_nibble(b[i + 1]), hex_nibble(b[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
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
