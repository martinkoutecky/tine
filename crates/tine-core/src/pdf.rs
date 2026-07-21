//! PDF highlight model + OG-compatible persistence.
//!
//! Two artifacts, matching Logseq:
//!  1. `assets/<key>.edn` — `{:highlights [...] :extra {}}` with scaled rects.
//!  2. `pages/hls__<key>.md` — an index page: a `file::`/`file-path::` pre-block
//!     plus one annotation block per highlight (`hl-page`, `hl-color`,
//!     `ls-type:: annotation`, `id`, and area-image `hl-stamp`). The block `id`
//!     equals the highlight id.

use crate::doc::{DocBlock, Document};
use crate::edn::{self, Edn};
use crate::model::Format;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub top: f64,
    pub left: f64,
    pub width: f64,
    pub height: f64,
    /// Coordinate-space dimensions used by current Logseq sidecars. `None`
    /// means the rectangle already uses the PDF's scale-1 page coordinates
    /// (the shape written by older Tine versions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_height: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub page: i64,
    pub bounding: Rect,
    pub rects: Vec<Rect>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Highlight {
    pub id: String,
    pub page: i64,
    pub position: Position,
    pub color: String,
    /// Selected text (text highlight) or None for area highlights.
    pub text: Option<String>,
    /// Image stamp (area highlight) or None for text highlights.
    pub image: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PdfState {
    pub highlights: Vec<Highlight>,
    pub page: Option<i64>,
    pub scale: Option<f64>,
}

// ----------------------------------------------------------------- EDN <-> model

fn kw(s: &str) -> Edn {
    Edn::Keyword(s.to_string())
}

fn finite(values: &[f64]) -> bool {
    values.iter().all(|value| value.is_finite())
}

fn rect_from(e: &Edn) -> Option<Rect> {
    if let (Some(top), Some(left), Some(width), Some(height)) = (
        e.get("top").and_then(Edn::as_f64),
        e.get("left").and_then(Edn::as_f64),
        e.get("width").and_then(Edn::as_f64),
        e.get("height").and_then(Edn::as_f64),
    ) {
        return (finite(&[top, left, width, height]) && width >= 0.0 && height >= 0.0).then_some(
            Rect {
                top,
                left,
                width,
                height,
                source_width: None,
                source_height: None,
            },
        );
    }

    // Logseq's current sidecars store rectangle corners as x1/y1/x2/y2; their
    // width/height fields describe the full PDF page rather than this rectangle.
    let left = e.get("x1")?.as_f64()?;
    let top = e.get("y1")?.as_f64()?;
    let right = e.get("x2")?.as_f64()?;
    let bottom = e.get("y2")?.as_f64()?;
    let source_width = e.get("width")?.as_f64()?;
    let source_height = e.get("height")?.as_f64()?;
    (finite(&[left, top, right, bottom, source_width, source_height])
        && right >= left
        && bottom >= top
        && source_width > 0.0
        && source_height > 0.0)
        .then_some(Rect {
            top,
            left,
            width: right - left,
            height: bottom - top,
            source_width: Some(source_width),
            source_height: Some(source_height),
        })
}

fn rect_to(r: &Rect) -> Edn {
    match (r.source_width, r.source_height) {
        (Some(source_width), Some(source_height))
            if source_width.is_finite()
                && source_height.is_finite()
                && source_width > 0.0
                && source_height > 0.0 =>
        {
            Edn::Map(vec![
                (kw("x1"), Edn::Float(r.left)),
                (kw("y1"), Edn::Float(r.top)),
                (kw("x2"), Edn::Float(r.left + r.width)),
                (kw("y2"), Edn::Float(r.top + r.height)),
                (kw("width"), Edn::Float(source_width)),
                (kw("height"), Edn::Float(source_height)),
            ])
        }
        _ => Edn::Map(vec![
            (kw("top"), Edn::Float(r.top)),
            (kw("left"), Edn::Float(r.left)),
            (kw("width"), Edn::Float(r.width)),
            (kw("height"), Edn::Float(r.height)),
        ]),
    }
}

fn highlight_id(e: &Edn) -> Option<&str> {
    match e {
        Edn::Str(id) => Some(id),
        Edn::Tagged(tag, value) if tag == "uuid" => match value.as_ref() {
            Edn::Str(id) => Some(id),
            _ => None,
        },
        _ => None,
    }
}

fn id_to(id: &str) -> Edn {
    if uuid::Uuid::parse_str(id).is_ok() {
        Edn::Tagged("uuid".to_string(), Box::new(Edn::Str(id.to_string())))
    } else {
        // Preserve compatibility with old/non-UUID fixtures rather than writing
        // an invalid #uuid reader literal that Logseq would reject.
        Edn::Str(id.to_string())
    }
}

fn highlight_from(e: &Edn) -> Option<Highlight> {
    let id = highlight_id(e.get("id")?)?.to_string();
    let page = e.get("page")?.as_i64()?;
    let pos = e.get("position")?;
    let position = Position {
        page: pos.get("page").and_then(Edn::as_i64).unwrap_or(page),
        bounding: rect_from(pos.get("bounding")?)?,
        rects: pos
            .get("rects")
            .and_then(Edn::as_vec)
            .map(|v| v.iter().filter_map(rect_from).collect())
            .unwrap_or_default(),
    };
    let color = e
        .get("properties")
        .and_then(|p| p.get("color"))
        .and_then(Edn::as_str)
        .unwrap_or("yellow")
        .to_string();
    let content = e.get("content");
    let mut text = content
        .and_then(|c| c.get("text"))
        .and_then(Edn::as_str)
        .map(String::from);
    let image = content.and_then(|c| c.get("image")).and_then(Edn::as_i64);
    // OG writes the "[:span]" sentinel for a new area highlight
    // (`extensions/pdf/core.cljs:491-506` at OG 6e7afa8eb). Older Tine sidecars
    // used an empty string. Neither is presentation text, so normalize both to
    // Tine's stable internal `None` representation.
    if image.is_some() && matches!(text.as_deref(), Some("" | "[:span]")) {
        text = None;
    }
    Some(Highlight {
        id,
        page,
        position,
        color,
        text,
        image,
    })
}

fn highlight_to(h: &Highlight) -> Edn {
    let position = Edn::Map(vec![
        (kw("page"), Edn::Int(h.position.page)),
        (kw("bounding"), rect_to(&h.position.bounding)),
        (
            kw("rects"),
            Edn::List(h.position.rects.iter().map(rect_to).collect()),
        ),
    ]);
    let mut content_pairs = Vec::new();
    if let Some(t) = &h.text {
        content_pairs.push((kw("text"), Edn::Str(t.clone())));
    } else if h.image.is_some() {
        // OG's new area map uses this exact sentinel
        // (`extensions/pdf/core.cljs:491-506` at OG 6e7afa8eb).
        content_pairs.push((kw("text"), Edn::Str("[:span]".to_string())));
    }
    if let Some(image) = h.image {
        content_pairs.push((kw("image"), Edn::Int(image)));
    }
    // OG's new text map contains only :text, with no nil :image key
    // (`extensions/pdf/core.cljs:636-653` at OG 6e7afa8eb).
    Edn::Map(vec![
        (kw("id"), id_to(&h.id)),
        (kw("page"), Edn::Int(h.page)),
        (kw("position"), position),
        (kw("content"), Edn::Map(content_pairs)),
        (
            kw("properties"),
            Edn::Map(vec![(kw("color"), Edn::Str(h.color.clone()))]),
        ),
    ])
}

/// Parse `assets/<key>.edn` contents into highlights.
pub fn parse_highlights(edn_str: &str) -> Vec<Highlight> {
    let Some(root) = edn::parse_strict(edn_str) else {
        return Vec::new();
    };
    root.get("highlights")
        .and_then(Edn::as_vec)
        .map(|v| v.iter().filter_map(highlight_from).collect())
        .unwrap_or_default()
}

/// Read highlights plus OG's persisted last-view page and scale. Unsupported
/// string scale modes such as `"auto"` intentionally map to `None`; Tine's
/// fit-width default is their visual equivalent and the original EDN remains
/// untouched until the user actually changes the view.
pub fn parse_pdf_state(edn_str: &str) -> PdfState {
    let Some(root) = edn::parse_strict(edn_str) else {
        return PdfState::default();
    };
    let highlights = root
        .get("highlights")
        .and_then(Edn::as_vec)
        .map(|v| v.iter().filter_map(highlight_from).collect())
        .unwrap_or_default();
    let extra = root.get("extra");
    let page = extra
        .and_then(|e| e.get("page"))
        .and_then(Edn::as_i64)
        .filter(|page| *page >= 1);
    let scale = extra
        .and_then(|e| e.get("scale"))
        .and_then(Edn::as_f64)
        .filter(|scale| scale.is_finite() && *scale > 0.0);
    PdfState {
        highlights,
        page,
        scale,
    }
}

/// Update only OG's `:extra` view fields while retaining highlights and all
/// foreign root/extra fields. Invalid existing EDN fails closed (`None`).
pub fn write_pdf_view_state(
    existing_edn: &str,
    page: i64,
    scale: f64,
) -> Option<String> {
    if page < 1 || !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let mut pairs = if existing_edn.trim().is_empty() {
        vec![(kw("highlights"), Edn::Vec(Vec::new()))]
    } else {
        match edn::parse_strict(existing_edn)? {
            Edn::Map(pairs) => pairs,
            _ => return None,
        }
    };
    let mut extra = match pairs
        .iter()
        .find(|(key, _)| matches!(key, Edn::Keyword(name) if name == "extra"))
        .map(|(_, value)| value)
    {
        Some(Edn::Map(existing)) => existing.clone(),
        _ => Vec::new(),
    };
    deep_merge(
        &mut extra,
        vec![(kw("page"), Edn::Int(page)), (kw("scale"), Edn::Float(scale))],
    );
    if let Some((_, value)) = pairs
        .iter_mut()
        .find(|(key, _)| matches!(key, Edn::Keyword(name) if name == "extra"))
    {
        *value = Edn::Map(extra);
    } else {
        pairs.push((kw("extra"), Edn::Map(extra)));
    }
    let mut out = edn::to_string(&Edn::Map(pairs));
    out.push('\n');
    Some(out)
}

/// Recursively merge `new` pairs onto `old` (in place): a key present in both whose
/// values are BOTH maps is merged deeper; otherwise `new` overwrites. Keys only in
/// `old` are kept. This is how foreign EDN (data Tine doesn't model) round-trips.
fn deep_merge(old: &mut Vec<(Edn, Edn)>, new: Vec<(Edn, Edn)>) {
    for (k, nv) in new {
        if let Some(pos) = old.iter().position(|(ek, _)| *ek == k) {
            match (&mut old[pos].1, nv) {
                (Edn::Map(om), Edn::Map(nm)) => deep_merge(om, nm),
                (slot, nv) => *slot = nv,
            }
        } else {
            old.push((k, nv));
        }
    }
}

/// Tine's fields for one highlight, merged ONTO its existing EDN map (matched by id)
/// so any keys the user/Logseq added — at the top level or inside content/properties —
/// survive a highlight edit. A brand-new highlight has no existing map → just ours.
fn merge_highlight(existing: Option<&Edn>, h: &Highlight) -> Edn {
    match (existing, highlight_to(h)) {
        (Some(Edn::Map(old)), Edn::Map(new)) => {
            let mut merged = old.clone();
            // `:position/:bounding` is deep-merged so foreign metadata survives,
            // but its old and current coordinate spellings must not coexist: an
            // old `:top` would otherwise shadow newly-written `:x1` on the next
            // read. `:rects` is replaced as a whole by deep_merge below.
            if let Some((_, Edn::Map(position))) = merged
                .iter_mut()
                .find(|(key, _)| matches!(key, Edn::Keyword(name) if name == "position"))
            {
                if let Some((_, Edn::Map(bounding))) = position
                    .iter_mut()
                    .find(|(key, _)| matches!(key, Edn::Keyword(name) if name == "bounding"))
                {
                    bounding.retain(|(key, _)| {
                        !matches!(
                            key,
                            Edn::Keyword(name)
                                if matches!(
                                    name.as_str(),
                                    "top" | "left" | "x1" | "y1" | "x2" | "y2" | "width" | "height"
                                )
                        )
                    });
                }
            }
            deep_merge(&mut merged, new);
            Edn::Map(merged)
        }
        (_, ours) => ours,
    }
}

/// Serialize highlights to `assets/<key>.edn`, PRESERVING the foreign content of the
/// existing file: only the `:highlights` Tine owns are replaced (each deep-merged onto
/// its prior map), while root `:extra`, any other root keys, and unknown per-highlight
/// fields round-trip untouched. Rebuilding from our model alone dropped all of it (audit
/// C#4: Logseq's `:extra`/metadata was silently erased on every highlight edit).
pub fn write_highlights(highlights: &[Highlight], existing_edn: &str) -> String {
    let root = edn::parse_strict(existing_edn);
    let existing_by_id: HashMap<String, Edn> = root
        .as_ref()
        .and_then(|r| r.get("highlights"))
        .and_then(Edn::as_vec)
        .map(|v| {
            v.iter()
                .filter_map(|h| Some((highlight_id(h.get("id")?)?.to_string(), h.clone())))
                .collect()
        })
        .unwrap_or_default();
    let hl_vec = Edn::Vec(
        highlights
            .iter()
            .map(|h| merge_highlight(existing_by_id.get(&h.id), h))
            .collect(),
    );

    // Keep every existing root key; replace only `:highlights`; ensure `:extra` exists
    // (OG canonical). A new/empty/unparseable file yields the canonical skeleton.
    let mut pairs: Vec<(Edn, Edn)> = match root {
        Some(Edn::Map(p)) => p,
        _ => Vec::new(),
    };
    let mut had_highlights = false;
    let mut had_extra = false;
    for (k, v) in pairs.iter_mut() {
        match k {
            Edn::Keyword(s) if s == "highlights" => {
                *v = hl_vec.clone();
                had_highlights = true;
            }
            Edn::Keyword(s) if s == "extra" => had_extra = true,
            _ => {}
        }
    }
    if !had_highlights {
        pairs.push((kw("highlights"), hl_vec));
    }
    if !had_extra {
        pairs.push((kw("extra"), Edn::Map(vec![])));
    }
    let mut s = edn::to_string(&Edn::Map(pairs));
    s.push('\n');
    s
}

// ----------------------------------------------------------------- hls__ page

/// Sanitize a PDF filename into the key used for the edn file + hls page.
///
/// Matches Logseq's `safe-sanitize-file-name` (the npm `sanitize-filename`
/// 1.6.3 library applied to the filename **stem**): only OS-illegal characters
/// are stripped — **case, `-`, `_`, spaces, and unicode letters are preserved**.
/// So `paper-1.pdf` and `paper_1.pdf` get *distinct* keys, exactly as OG keeps
/// them; `My Paper.pdf` → key `My Paper`. (Tine's old key lowercased and mapped
/// every non-alphanumeric to `_`, which diverged from OG and collided distinct
/// PDFs — see `legacy_asset_key` for the read-fallback/migration path.)
pub fn asset_key(pdf_filename: &str) -> String {
    let stem = pdf_filename
        .strip_suffix(".pdf")
        .or_else(|| pdf_filename.strip_suffix(".PDF"))
        .unwrap_or(pdf_filename);
    sanitize_filename(stem)
}

/// Strip only the characters the npm `sanitize-filename` 1.6.3 library removes
/// (default empty-string replacement), so the result matches OG's key byte-for-
/// byte: drop the reserved set `/ ? < > \ : * | "`, control chars
/// (`0x00–0x1f`, `0x80–0x9f`), trailing dots/spaces, and Windows reserved device
/// names (CON/PRN/AUX/NUL/COM0-9/LPT0-9). Nothing is lowercased or substituted.
fn sanitize_filename(s: &str) -> String {
    let illegal = |c: char| matches!(c, '/' | '?' | '<' | '>' | '\\' | ':' | '*' | '|' | '"');
    let control = |c: char| {
        let n = c as u32;
        n <= 0x1f || (0x80..=0x9f).contains(&n)
    };
    let mut out: String = s.chars().filter(|&c| !illegal(c) && !control(c)).collect();
    // Windows: strip trailing dots and spaces.
    while out.ends_with('.') || out.ends_with(' ') {
        out.pop();
    }
    // Windows reserved device names (optionally with an extension) → removed.
    let base = out.split('.').next().unwrap_or("").to_ascii_lowercase();
    let reserved = matches!(base.as_str(), "con" | "prn" | "aux" | "nul")
        || ((base.starts_with("com") || base.starts_with("lpt"))
            && base.len() == 4
            && base.as_bytes()[3].is_ascii_digit());
    if reserved {
        out.clear();
    }
    out
}

/// Tine's pre-launch key scheme (lowercased, every non-alphanumeric → `_`).
/// Retained ONLY so highlight files written by older Tine builds can still be
/// located (read-fallback) and migrated forward to the OG-compatible
/// [`asset_key`] on the next write. Do not use for new writes.
pub fn legacy_asset_key(pdf_filename: &str) -> String {
    let stem = pdf_filename.strip_suffix(".pdf").unwrap_or(pdf_filename);
    stem.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

pub fn hls_page_name(key: &str) -> String {
    format!("hls__{key}")
}

/// Build the `hls__<key>` index page document for a set of highlights.
pub fn hls_page_document(pdf_filename: &str, label: &str, highlights: &[Highlight]) -> Document {
    hls_page_document_for_format(pdf_filename, label, highlights, Format::Md)
}

pub fn hls_page_document_for_format(
    pdf_filename: &str,
    label: &str,
    highlights: &[Highlight],
    format: Format,
) -> Document {
    merge_hls_page_for_format(None, pdf_filename, label, highlights, format)
}

/// Upsert highlights into an existing `hls__` page, **preserving each existing
/// annotation block (and its child notes) by `id`**. New highlights are
/// appended; highlights deleted from the set drop their block. This is what
/// makes the review flow safe — re-saving never clobbers notes.
pub fn merge_hls_page(
    existing: Option<&Document>,
    pdf_filename: &str,
    label: &str,
    highlights: &[Highlight],
) -> Document {
    merge_hls_page_for_format(existing, pdf_filename, label, highlights, Format::Md)
}

pub fn merge_hls_page_for_format(
    existing: Option<&Document>,
    pdf_filename: &str,
    label: &str,
    highlights: &[Highlight],
    format: Format,
) -> Document {
    let asset_path = format!("../assets/{pdf_filename}");
    // Start with the generated file::/file-path::, then keep any OTHER pre-block
    // properties the user added (e.g. tags::) — replacing only the generated two
    // rather than discarding the whole pre-block.
    let mut pre_lines = match format {
        Format::Md => vec![
            format!("file:: [{label}]({asset_path})"),
            format!("file-path:: {asset_path}"),
        ],
        Format::Org => vec![
            format!("#+FILE: [[{asset_path}][{label}]]"),
            format!("#+FILE-PATH: {asset_path}"),
        ],
    };
    if let Some(doc) = existing {
        if let Some(prev) = &doc.pre_block {
            for line in prev.lines() {
                let key = page_property_key(line, format);
                if matches!(key.as_deref(), Some("file") | Some("file-path")) {
                    continue;
                }
                pre_lines.push(line.to_string());
            }
        }
    }
    let pre = pre_lines.join("\n");

    // Split existing roots into annotation blocks (keyed by id) and everything
    // else — user-authored top-level notes that must be PRESERVED, not rebuilt
    // away. Annotations are regenerated from the (authoritative) highlight list so
    // a removed highlight drops its block; a non-annotation root is always kept.
    let mut ann_by_id: HashMap<String, DocBlock> = HashMap::new();
    let mut user_roots: Vec<DocBlock> = Vec::new();
    if let Some(doc) = existing {
        for b in &doc.roots {
            let is_annotation = b.property("ls-type").as_deref() == Some("annotation");
            match b.property("id") {
                Some(id) if is_annotation => {
                    ann_by_id.insert(id, b.clone());
                }
                _ => user_roots.push(b.clone()),
            }
        }
    }

    let mut roots: Vec<DocBlock> = highlights
        .iter()
        .map(|h| match ann_by_id.remove(&h.id) {
            // Keep the user's note text + child blocks, but refresh the highlight
            // metadata (color/page) from the authoritative highlight — so
            // recoloring in the PDF pane updates the colored badge here too.
            Some(existing) => refresh_annotation(existing, h, format),
            None => highlight_block(h, format),
        })
        .collect();
    // Keep the user's own top-level notes (after the generated annotations).
    roots.extend(user_roots);
    Document {
        pre_block: Some(pre),
        roots,
    }
}

fn page_property_key(line: &str, format: Format) -> Option<String> {
    match format {
        Format::Md => crate::doc::parse_property_line(line).map(|(k, _)| k.to_ascii_lowercase()),
        Format::Org => line
            .trim()
            .strip_prefix("#+")
            .and_then(|line| line.split_once(':'))
            .map(|(key, _)| key.to_ascii_lowercase()),
    }
}

fn block_property_key(line: &str, format: Format) -> Option<String> {
    match format {
        Format::Md => crate::doc::parse_property_line(line).map(|(k, _)| k.to_ascii_lowercase()),
        Format::Org => {
            let line = line.trim();
            if line.eq_ignore_ascii_case(":PROPERTIES:") || line.eq_ignore_ascii_case(":END:") {
                return None;
            }
            line.strip_prefix(':')
                .and_then(|line| line.split_once(':'))
                .map(|(key, _)| key.to_ascii_lowercase())
        }
    }
}

fn property_line(key: &str, value: impl std::fmt::Display, format: Format) -> String {
    match format {
        Format::Md => format!("{key}:: {value}"),
        Format::Org => format!(":{key}: {value}"),
    }
}

/// Refresh an existing annotation block's `hl-color::` / `hl-page::` to match the
/// (possibly recolored / re-paged) highlight, preserving everything else — the
/// user's highlight-text line, any extra properties, and the note children. The
/// old block was previously kept verbatim, so a recolor never reached the page.
fn refresh_annotation(mut block: DocBlock, h: &Highlight, format: Format) -> DocBlock {
    let mut saw_color = false;
    let mut saw_page = false;
    let mut lines: Vec<String> = block
        .raw
        .lines()
        .map(|line| {
            match block_property_key(line, format) {
                Some(k) if k == "hl-color" => {
                    saw_color = true;
                    property_line("hl-color", &h.color, format)
                }
                Some(k) if k == "hl-page" => {
                    saw_page = true;
                    property_line("hl-page", h.page, format)
                }
                _ => line.to_string(),
            }
        })
        .collect();
    // If the metadata lines were missing (hand-edited file), add them before id::.
    if !saw_color || !saw_page {
        let id_pos = lines
            .iter()
            .position(|line| block_property_key(line, format).as_deref() == Some("id"));
        let mut add: Vec<String> = Vec::new();
        if !saw_page {
            add.push(property_line("hl-page", h.page, format));
        }
        if !saw_color {
            add.push(property_line("hl-color", &h.color, format));
        }
        match id_pos {
            Some(i) => {
                for (j, l) in add.into_iter().enumerate() {
                    lines.insert(i + j, l);
                }
            }
            None if format == Format::Org => {
                let end = lines
                    .iter()
                    .position(|line| line.trim().eq_ignore_ascii_case(":END:"));
                match end {
                    Some(index) => {
                        for (offset, line) in add.into_iter().enumerate() {
                            lines.insert(index + offset, line);
                        }
                    }
                    None => {
                        lines.push(":PROPERTIES:".to_string());
                        lines.extend(add);
                        lines.push(":END:".to_string());
                    }
                }
            }
            None => lines.extend(add),
        }
    }
    block.raw = lines.join("\n");
    block
}

fn highlight_block(h: &Highlight, format: Format) -> DocBlock {
    let mut lines = Vec::new();
    if let Some(t) = &h.text {
        lines.push(t.clone());
    } else {
        lines.push(String::new());
    }
    if format == Format::Org {
        lines.push(":PROPERTIES:".to_string());
    }
    lines.push(property_line("hl-page", h.page, format));
    lines.push(property_line("hl-color", &h.color, format));
    if let Some(image_stamp) = h.image {
        lines.push(property_line("hl-type", "area", format));
        // OG's file-graph writer copies :content.image verbatim into hl-stamp.
        // Text highlights have no image stamp and omit both area properties.
        lines.push(property_line("hl-stamp", image_stamp, format));
    }
    lines.push(property_line("ls-type", "annotation", format));
    lines.push(property_line("id", &h.id, format));
    if format == Format::Org {
        lines.push(":END:".to_string());
    }
    DocBlock {
        raw: lines.join("\n"),
        children: Vec::new(),
        uuid: String::new(),
        is_org: format == Format::Org,
        proj: std::sync::OnceLock::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Highlight {
        Highlight {
            id: "5e8f9c7b-1234-5678-abcd-ef1234567890".into(),
            page: 42,
            position: Position {
                page: 42,
                bounding: Rect {
                    top: 100.0,
                    left: 50.0,
                    width: 400.0,
                    height: 200.0,
                    source_width: None,
                    source_height: None,
                },
                rects: vec![Rect {
                    top: 100.0,
                    left: 50.0,
                    width: 400.0,
                    height: 20.0,
                    source_width: None,
                    source_height: None,
                }],
            },
            color: "yellow".into(),
            text: Some("some highlighted text".into()),
            image: None,
        }
    }

    #[test]
    fn highlights_edn_roundtrip() {
        let hs = vec![sample()];
        let edn_str = write_highlights(&hs, "");
        let parsed = parse_highlights(&edn_str);
        assert_eq!(parsed, hs);
    }

    #[test]
    fn new_text_highlight_omits_nil_image_key() {
        let out = write_highlights(&[sample()], "");
        let root = edn::parse_strict(&out).unwrap();
        let stored = &root.get("highlights").and_then(Edn::as_vec).unwrap()[0];
        let content = stored.get("content").unwrap();
        assert_eq!(
            content.get("text").and_then(Edn::as_str),
            Some("some highlighted text")
        );
        assert_eq!(
            content.get("image"),
            None,
            "new OG-shaped text highlight: {out}"
        );
    }

    #[test]
    fn write_preserves_foreign_edn() {
        // audit C#4: editing a highlight must NOT drop Logseq's root :extra / other root
        // keys / unknown per-highlight fields.
        let existing = r#"{:highlights [{:id "abc" :page 3
            :position {:page 3 :bounding {:top 10 :left 20 :width 30 :height 40} :rects []}
            :content {:text "hi"} :properties {:color "green"}
            :ls-mystery "keep-me"}]
            :extra {:zoom 1.5} :future-root-key 42}"#;
        let mut h = sample();
        h.id = "abc".into();
        h.color = "red".into(); // an edit Tine owns
        let out = write_highlights(&[h], existing);
        let root = crate::edn::parse(&out).unwrap();
        // root :extra and the unknown root key survive
        assert_eq!(
            root.get("extra")
                .and_then(|e| e.get("zoom"))
                .and_then(crate::edn::Edn::as_f64),
            Some(1.5)
        );
        assert_eq!(
            root.get("future-root-key")
                .and_then(crate::edn::Edn::as_i64),
            Some(42)
        );
        // the unknown per-highlight field survives, and Tine's edit applied
        let hl = &root
            .get("highlights")
            .and_then(crate::edn::Edn::as_vec)
            .unwrap()[0];
        assert_eq!(
            hl.get("ls-mystery").and_then(crate::edn::Edn::as_str),
            Some("keep-me")
        );
        assert_eq!(
            hl.get("properties")
                .and_then(|p| p.get("color"))
                .and_then(crate::edn::Edn::as_str),
            Some("red")
        );
    }

    #[test]
    fn parses_og_shaped_edn() {
        let src = r#"{:highlights [{:id "abc" :page 3
            :position {:page 3 :bounding {:top 10 :left 20 :width 30 :height 40}
                       :rects [{:top 10 :left 20 :width 30 :height 12}]}
            :content {:text "hello"} :properties {:color "green"}}] :extra {}}"#;
        let hs = parse_highlights(src);
        assert_eq!(hs.len(), 1);
        assert_eq!(hs[0].id, "abc");
        assert_eq!(hs[0].color, "green");
        assert_eq!(hs[0].text.as_deref(), Some("hello"));
        assert_eq!(hs[0].position.rects.len(), 1);
    }

    #[test]
    fn parses_current_logseq_uuid_list_and_corner_rect_shape() {
        let src = r#"{:highlights [{:id #uuid "6a5604f8-a337-4336-a711-2ba6bc14fbfd"
            :page 1
            :position {:bounding {:x1 292.1 :y1 488.4 :x2 555.5 :y2 535.1
                                  :width 822 :height 1063.7}
                       :rects ({:x1 292.1 :y1 488.4 :x2 555.5 :y2 535.1
                                :width 822 :height 1063.7})
                       :page 1}
            :content {:text "MyLifeOrganized"}
            :properties {:color "yellow"}}]
            :extra {:page 1}}"#;
        let hs = parse_highlights(src);
        assert_eq!(hs.len(), 1);
        assert_eq!(hs[0].id, "6a5604f8-a337-4336-a711-2ba6bc14fbfd");
        assert_eq!(hs[0].position.bounding.left, 292.1);
        assert!((hs[0].position.bounding.width - 263.4).abs() < 1e-9);
        assert_eq!(hs[0].position.bounding.source_width, Some(822.0));
        assert_eq!(hs[0].position.bounding.source_height, Some(1063.7));
        assert_eq!(hs[0].position.rects.len(), 1);
        assert!((hs[0].position.rects[0].height - 46.7).abs() < 1e-9);
    }

    #[test]
    fn writes_current_logseq_uuid_list_and_corner_rect_shape() {
        let mut h = sample();
        for rect in std::iter::once(&mut h.position.bounding).chain(h.position.rects.iter_mut()) {
            rect.source_width = Some(612.0);
            rect.source_height = Some(792.0);
        }
        let out = write_highlights(&[h.clone()], "");
        assert!(
            out.contains(r#":id #uuid "5e8f9c7b-1234-5678-abcd-ef1234567890""#),
            "{out}"
        );
        assert!(out.contains(":rects ("), "{out}");
        assert!(out.contains(":x1 50"), "{out}");
        assert!(out.contains(":x2 450"), "{out}");
        assert_eq!(parse_highlights(&out), vec![h]);
    }

    #[test]
    fn new_area_highlight_writes_og_span_sentinel_and_empty_rect_list() {
        let mut h = sample();
        h.text = None;
        h.image = Some(1659920114630);
        h.position.rects.clear();
        let out = write_highlights(&[h.clone()], "");
        let root = edn::parse_strict(&out).unwrap();
        let stored = &root.get("highlights").and_then(Edn::as_vec).unwrap()[0];
        assert_eq!(
            stored
                .get("content")
                .unwrap()
                .get("text")
                .and_then(Edn::as_str),
            Some("[:span]")
        );
        assert!(stored
            .get("position")
            .unwrap()
            .get("rects")
            .and_then(Edn::as_vec)
            .unwrap()
            .is_empty());
        assert_eq!(parse_highlights(&out), vec![h]);
    }

    #[test]
    fn pdf_state_reads_and_updates_og_extra_without_touching_foreign_data() {
        let existing = r#"{:highlights [] :extra {:page 7 :scale 1.75 :plugin "keep"} :future 42}"#;
        let state = parse_pdf_state(existing);
        assert_eq!(state.page, Some(7));
        assert_eq!(state.scale, Some(1.75));

        let out = write_pdf_view_state(existing, 9, 2.25).unwrap();
        let root = edn::parse_strict(&out).unwrap();
        let extra = root.get("extra").unwrap();
        assert_eq!(extra.get("page").and_then(Edn::as_i64), Some(9));
        assert_eq!(extra.get("scale").and_then(Edn::as_f64), Some(2.25));
        assert_eq!(extra.get("plugin").and_then(Edn::as_str), Some("keep"));
        assert_eq!(root.get("future").and_then(Edn::as_i64), Some(42));
    }

    #[test]
    fn geometry_migration_removes_shadowing_old_keys_but_keeps_foreign_metadata() {
        let existing = r#"{:highlights [{:id #uuid "5e8f9c7b-1234-5678-abcd-ef1234567890"
          :page 42 :position {:page 42
            :bounding {:top 1 :left 2 :width 3 :height 4 :plugin-note "keep"}
            :rects [{:top 1 :left 2 :width 3 :height 4}]}
          :content {:text "old"} :properties {:color "yellow"}}] :extra {}}"#;
        let mut h = sample();
        for rect in std::iter::once(&mut h.position.bounding).chain(h.position.rects.iter_mut()) {
            rect.source_width = Some(612.0);
            rect.source_height = Some(792.0);
        }
        let out = write_highlights(&[h.clone()], existing);
        let root = edn::parse_strict(&out).unwrap();
        let stored = &root.get("highlights").and_then(Edn::as_vec).unwrap()[0];
        let bounding = stored.get("position").unwrap().get("bounding").unwrap();
        assert_eq!(bounding.get("top"), None, "old geometry survived: {out}");
        assert_eq!(
            bounding.get("plugin-note").and_then(Edn::as_str),
            Some("keep")
        );
        assert_eq!(parse_highlights(&out), vec![h]);
    }

    #[test]
    fn hls_page_has_annotation_blocks() {
        let doc = hls_page_document("my-book.pdf", "My Book", &[sample()]);
        let md = crate::doc::serialize(&doc);
        assert!(md.contains("file-path:: ../assets/my-book.pdf"));
        assert!(md.contains("ls-type:: annotation"));
        assert!(md.contains("hl-page:: 42"));
        assert!(md.contains("hl-color:: yellow"));
        assert!(md.contains("id:: 5e8f9c7b-1234-5678-abcd-ef1234567890"));
        // The block round-trips through the markdown parser.
        let back = crate::doc::parse(&md);
        assert_eq!(back.roots.len(), 1);
        assert_eq!(back.roots[0].property("hl-page").as_deref(), Some("42"));
    }

    #[test]
    fn org_hls_page_uses_og_page_properties_and_annotation_drawer() {
        let doc = hls_page_document_for_format(
            "my-book.pdf",
            "My Book",
            &[sample()],
            crate::model::Format::Org,
        );
        let org = crate::org::serialize_org(&doc);
        assert!(org.contains("#+FILE: [[../assets/my-book.pdf][My Book]]"), "{org}");
        assert!(org.contains("#+FILE-PATH: ../assets/my-book.pdf"), "{org}");
        assert!(org.contains("* some highlighted text"), "{org}");
        assert!(org.contains(":PROPERTIES:"), "{org}");
        assert!(org.contains(":hl-page: 42"), "{org}");
        assert!(org.contains(":ls-type: annotation"), "{org}");
        assert!(org.contains(":id: 5e8f9c7b-1234-5678-abcd-ef1234567890"), "{org}");
        assert!(crate::org::org_round_trips(&org));
        let parsed = crate::org::parse_org(&org);
        assert_eq!(parsed.roots[0].property("hl-page").as_deref(), Some("42"));
    }

    #[test]
    fn merge_preserves_user_top_level_notes() {
        let h = sample();
        // Existing page: a user's own top-level note + one annotation block.
        let existing = crate::doc::parse(&format!(
            "- my summary note\n- highlighted text\n  ls-type:: annotation\n  id:: {}\n",
            h.id
        ));
        let merged = merge_hls_page(Some(&existing), "paper.pdf", "Paper", &[h.clone()]);
        let raws: Vec<&str> = merged.roots.iter().map(|b| b.raw.as_str()).collect();
        assert!(
            raws.iter().any(|r| r.contains("my summary note")),
            "user note dropped: {raws:?}"
        );
        assert!(
            raws.iter().any(|r| r.contains(&h.id)),
            "annotation missing: {raws:?}"
        );
    }

    #[test]
    fn merge_hls_page_keeps_user_page_properties() {
        // Existing hls page with a user property (tags::) + stale file::/file-path::.
        let existing = crate::doc::parse(
            "tags:: reading\nfile:: [old](../assets/old.pdf)\nfile-path:: ../assets/old.pdf\n",
        );
        let merged = merge_hls_page(Some(&existing), "paper.pdf", "Paper", &[]);
        let pre = merged.pre_block.unwrap();
        assert!(
            pre.contains("tags:: reading"),
            "user page property dropped: {pre}"
        );
        assert!(
            pre.contains("file-path:: ../assets/paper.pdf"),
            "file-path not updated: {pre}"
        );
        assert!(!pre.contains("old.pdf"), "stale file path kept: {pre}");
    }

    #[test]
    fn merge_preserves_note_children() {
        // First save: one highlight.
        let h1 = sample();
        let page1 = hls_page_document("b.pdf", "b", &[h1.clone()]);
        // User adds a note child under the highlight, then edits on disk.
        let mut md = crate::doc::serialize(&page1);
        md.push_str("\t- my note about this highlight\n");
        let edited = crate::doc::parse(&md);
        assert_eq!(edited.roots[0].children.len(), 1);

        // Second save: same highlight + a new one. Note must survive.
        let mut h2 = sample();
        h2.id = "new-id-2".into();
        h2.text = Some("second highlight".into());
        let merged = merge_hls_page(Some(&edited), "b.pdf", "b", &[h1, h2]);
        assert_eq!(merged.roots.len(), 2);
        assert_eq!(merged.roots[0].children.len(), 1, "note child preserved");
        assert_eq!(
            merged.roots[0].children[0].raw,
            "my note about this highlight"
        );
        assert!(merged.roots[1].raw.contains("second highlight"));
    }

    #[test]
    fn merge_updates_color_on_recolor() {
        // First save: yellow highlight, with a user note child.
        let mut h = sample();
        h.color = "yellow".into();
        let page = hls_page_document("b.pdf", "b", &[h.clone()]);
        let mut md = crate::doc::serialize(&page);
        md.push_str("\t- my note\n");
        let edited = crate::doc::parse(&md);

        // Recolor to green and re-merge.
        h.color = "green".into();
        let merged = merge_hls_page(Some(&edited), "b.pdf", "b", &[h]);
        assert_eq!(merged.roots.len(), 1);
        assert!(
            merged.roots[0].raw.contains("hl-color:: green"),
            "color not updated: {}",
            merged.roots[0].raw
        );
        assert!(
            !merged.roots[0].raw.contains("hl-color:: yellow"),
            "old color kept: {}",
            merged.roots[0].raw
        );
        assert_eq!(
            merged.roots[0].children.len(),
            1,
            "note child must survive a recolor"
        );
        assert_eq!(merged.roots[0].children[0].raw, "my note");
    }

    #[test]
    fn merge_drops_deleted_highlights() {
        let h1 = sample();
        let page = hls_page_document("b.pdf", "b", &[h1]);
        // Re-save with an empty highlight set → block removed.
        let merged = merge_hls_page(Some(&page), "b.pdf", "b", &[]);
        assert_eq!(merged.roots.len(), 0);
    }

    #[test]
    fn asset_key_matches_og_sanitize_filename() {
        // Case + `-`/`_`/spaces preserved; `-` and `_` stay DISTINCT (no collision).
        assert_eq!(asset_key("paper-1.pdf"), "paper-1");
        assert_eq!(asset_key("paper_1.pdf"), "paper_1");
        assert_ne!(asset_key("paper-1.pdf"), asset_key("paper_1.pdf"));
        assert_eq!(asset_key("My Paper.pdf"), "My Paper");
        // OS-illegal chars dropped (empty replacement), case otherwise kept.
        assert_eq!(asset_key("a/b:c.pdf"), "abc");
        assert_eq!(asset_key("Re: notes?.pdf"), "Re notes");
        // Uppercase extension handled.
        assert_eq!(asset_key("Book.PDF"), "Book");
        // Trailing dots/spaces stripped; reserved device name removed.
        assert_eq!(asset_key("draft. .pdf"), "draft");
        assert_eq!(asset_key("CON.pdf"), "");
    }

    #[test]
    fn legacy_key_is_the_old_lowercase_scheme() {
        assert_eq!(legacy_asset_key("paper-1.pdf"), "paper_1");
        assert_eq!(legacy_asset_key("My Paper.pdf"), "my_paper");
        // Already-simple names agree with the new key (no migration needed).
        assert_eq!(legacy_asset_key("paper.pdf"), asset_key("paper.pdf"));
    }

    #[test]
    fn text_highlight_omits_area_metadata() {
        let md = crate::doc::serialize(&hls_page_document("x.pdf", "x", &[sample()]));
        assert!(!md.contains("hl-type:: area"));
        assert!(!md.contains("hl-stamp::"));
    }

    #[test]
    fn area_highlight_uses_image_as_hl_stamp() {
        let mut h = sample();
        h.text = None;
        h.image = Some(1659920114630);
        let md = crate::doc::serialize(&hls_page_document("x.pdf", "x", &[h]));
        assert!(md.contains("hl-type:: area"));
        assert!(md.contains("hl-stamp:: 1659920114630"));
    }

    #[test]
    fn merge_preserves_existing_annotation_properties() {
        let h = sample();
        let existing = crate::doc::parse(&format!(
            "- highlighted text\n  hl-page:: 1\n  hl-color:: red\n  ls-type:: annotation\n  id:: {}\n  hl-stamp:: foreign-value\n  plugin-data:: keep-me\n",
            h.id
        ));

        let merged = merge_hls_page(Some(&existing), "x.pdf", "x", &[h]);
        let raw = &merged.roots[0].raw;
        assert!(raw.contains("hl-stamp:: foreign-value"), "{raw}");
        assert!(raw.contains("plugin-data:: keep-me"), "{raw}");
    }
}
