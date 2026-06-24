//! PDF highlight model + OG-compatible persistence.
//!
//! Two artifacts, matching Logseq:
//!  1. `assets/<key>.edn` — `{:highlights [...] :extra {}}` with scaled rects.
//!  2. `pages/hls__<key>.md` — an index page: a `file::`/`file-path::` pre-block
//!     plus one annotation block per highlight (`hl-page`, `hl-color`,
//!     `ls-type:: annotation`, `id`). The block `id` equals the highlight id.

use crate::doc::{DocBlock, Document};
use crate::edn::{self, Edn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub top: f64,
    pub left: f64,
    pub width: f64,
    pub height: f64,
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

// ----------------------------------------------------------------- EDN <-> model

fn kw(s: &str) -> Edn {
    Edn::Keyword(s.to_string())
}

fn rect_from(e: &Edn) -> Option<Rect> {
    Some(Rect {
        top: e.get("top")?.as_f64()?,
        left: e.get("left")?.as_f64()?,
        width: e.get("width")?.as_f64()?,
        height: e.get("height")?.as_f64()?,
    })
}

fn rect_to(r: &Rect) -> Edn {
    Edn::Map(vec![
        (kw("top"), Edn::Float(r.top)),
        (kw("left"), Edn::Float(r.left)),
        (kw("width"), Edn::Float(r.width)),
        (kw("height"), Edn::Float(r.height)),
    ])
}

fn highlight_from(e: &Edn) -> Option<Highlight> {
    let id = e.get("id")?.as_str()?.to_string();
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
    let text = content.and_then(|c| c.get("text")).and_then(Edn::as_str).map(String::from);
    let image = content.and_then(|c| c.get("image")).and_then(Edn::as_i64);
    Some(Highlight { id, page, position, color, text, image })
}

fn highlight_to(h: &Highlight) -> Edn {
    let position = Edn::Map(vec![
        (kw("page"), Edn::Int(h.position.page)),
        (kw("bounding"), rect_to(&h.position.bounding)),
        (kw("rects"), Edn::Vec(h.position.rects.iter().map(rect_to).collect())),
    ]);
    let mut content_pairs = Vec::new();
    if let Some(t) = &h.text {
        content_pairs.push((kw("text"), Edn::Str(t.clone())));
    }
    content_pairs.push((kw("image"), h.image.map(Edn::Int).unwrap_or(Edn::Nil)));
    Edn::Map(vec![
        (kw("id"), Edn::Str(h.id.clone())),
        (kw("page"), Edn::Int(h.page)),
        (kw("position"), position),
        (kw("content"), Edn::Map(content_pairs)),
        (kw("properties"), Edn::Map(vec![(kw("color"), Edn::Str(h.color.clone()))])),
    ])
}

/// Parse `assets/<key>.edn` contents into highlights.
pub fn parse_highlights(edn_str: &str) -> Vec<Highlight> {
    let Some(root) = edn::parse(edn_str) else { return Vec::new() };
    root.get("highlights")
        .and_then(Edn::as_vec)
        .map(|v| v.iter().filter_map(highlight_from).collect())
        .unwrap_or_default()
}

/// Serialize highlights to `assets/<key>.edn` contents.
pub fn write_highlights(highlights: &[Highlight]) -> String {
    let root = Edn::Map(vec![
        (kw("highlights"), Edn::Vec(highlights.iter().map(highlight_to).collect())),
        (kw("extra"), Edn::Map(vec![])),
    ]);
    let mut s = edn::to_string(&root);
    s.push('\n');
    s
}

// ----------------------------------------------------------------- hls__ page

/// Sanitize a PDF filename into the key used for the edn file + hls page.
///
/// Matches Logseq's `safe-sanitize-file-name` (the npm `sanitize-filename`
/// 1.6.4 library applied to the filename **stem**): only OS-illegal characters
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

/// Strip only the characters the npm `sanitize-filename` 1.6.4 library removes
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
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect()
}

pub fn hls_page_name(key: &str) -> String {
    format!("hls__{key}")
}

/// Build the `hls__<key>` index page document for a set of highlights.
pub fn hls_page_document(pdf_filename: &str, label: &str, highlights: &[Highlight]) -> Document {
    merge_hls_page(None, pdf_filename, label, highlights)
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
    let asset_path = format!("../assets/{pdf_filename}");
    // Start with the generated file::/file-path::, then keep any OTHER pre-block
    // properties the user added (e.g. tags::) — replacing only the generated two
    // rather than discarding the whole pre-block.
    let mut pre_lines = vec![
        format!("file:: [{label}]({asset_path})"),
        format!("file-path:: {asset_path}"),
    ];
    if let Some(doc) = existing {
        if let Some(prev) = &doc.pre_block {
            for line in prev.lines() {
                let key = crate::doc::parse_property_line(line).map(|(k, _)| k.to_ascii_lowercase());
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
        .map(|h| ann_by_id.remove(&h.id).unwrap_or_else(|| highlight_block(h)))
        .collect();
    // Keep the user's own top-level notes (after the generated annotations).
    roots.extend(user_roots);
    Document { pre_block: Some(pre), roots }
}

fn highlight_block(h: &Highlight) -> DocBlock {
    let mut lines = Vec::new();
    if let Some(t) = &h.text {
        lines.push(t.clone());
    }
    lines.push(format!("hl-page:: {}", h.page));
    lines.push(format!("hl-color:: {}", h.color));
    if h.image.is_some() {
        lines.push("hl-type:: area".to_string());
    }
    lines.push("ls-type:: annotation".to_string());
    lines.push(format!("id:: {}", h.id));
    DocBlock { raw: lines.join("\n"), children: Vec::new(), uuid: String::new() }
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
                bounding: Rect { top: 100.0, left: 50.0, width: 400.0, height: 200.0 },
                rects: vec![Rect { top: 100.0, left: 50.0, width: 400.0, height: 20.0 }],
            },
            color: "yellow".into(),
            text: Some("some highlighted text".into()),
            image: None,
        }
    }

    #[test]
    fn highlights_edn_roundtrip() {
        let hs = vec![sample()];
        let edn_str = write_highlights(&hs);
        let parsed = parse_highlights(&edn_str);
        assert_eq!(parsed, hs);
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
    fn merge_preserves_user_top_level_notes() {
        let h = sample();
        // Existing page: a user's own top-level note + one annotation block.
        let existing = crate::doc::parse(&format!(
            "- my summary note\n- highlighted text\n  ls-type:: annotation\n  id:: {}\n",
            h.id
        ));
        let merged = merge_hls_page(Some(&existing), "paper.pdf", "Paper", &[h.clone()]);
        let raws: Vec<&str> = merged.roots.iter().map(|b| b.raw.as_str()).collect();
        assert!(raws.iter().any(|r| r.contains("my summary note")), "user note dropped: {raws:?}");
        assert!(raws.iter().any(|r| r.contains(&h.id)), "annotation missing: {raws:?}");
    }

    #[test]
    fn merge_hls_page_keeps_user_page_properties() {
        // Existing hls page with a user property (tags::) + stale file::/file-path::.
        let existing = crate::doc::parse(
            "tags:: reading\nfile:: [old](../assets/old.pdf)\nfile-path:: ../assets/old.pdf\n",
        );
        let merged = merge_hls_page(Some(&existing), "paper.pdf", "Paper", &[]);
        let pre = merged.pre_block.unwrap();
        assert!(pre.contains("tags:: reading"), "user page property dropped: {pre}");
        assert!(pre.contains("file-path:: ../assets/paper.pdf"), "file-path not updated: {pre}");
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
        assert_eq!(merged.roots[0].children[0].raw, "my note about this highlight");
        assert!(merged.roots[1].raw.contains("second highlight"));
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
    fn area_highlight_marks_type() {
        let mut h = sample();
        h.text = None;
        h.image = Some(1659920114630);
        let md = crate::doc::serialize(&hls_page_document("x.pdf", "x", &[h]));
        assert!(md.contains("hl-type:: area"));
    }
}
