//! Core Spotlight index of the current graph (iOS; Martin 2026-10-10).
//!
//! Each page becomes one searchable item: its title plus a short excerpt,
//! addressed by its page key and opening `tine://page/<name>` (the S3 route
//! for the current graph, `src/deepLinks.ts`). Only the main window's graph is
//! indexed. The source is the published whole-graph view, so a page the
//! app's own visibility rules keep from every other surface (`:hidden`
//! directories, trash, version files, conflict copies) is never indexed:
//! discovery (`graph_text_relative_eligible`) already left it out.
//!
//! - graph bound and warm: replace the whole index (`reindex`), O(P) once
//!   per graph load;
//! - every publication, own or external (watcher.rs `dispatch`): upsert or
//!   delete the changed pages only (`observe`), O(changed pages);
//! - graph switched in the window, or forgotten: clear (`clear`, `forget`).
//!
//! Spotlight is a cache: every failure is logged and otherwise ignored.
use serde::Serialize;
use std::sync::Mutex;
use tine_core::model::PageKind;

/// The longest excerpt shown under a result, in characters.
const EXCERPT_CHARS: usize = 120;
/// Only this window's graph is "the current graph" (iOS has no other).
const INDEXED_WINDOW: &str = "main";

/// Root of the graph whose pages are in the index now.
static INDEXED_ROOT: Mutex<Option<String>> = Mutex::new(None);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Entry {
    /// Stable within a graph: the page key, so renames and deletes address it.
    pub id: String,
    pub title: String,
    pub excerpt: String,
    /// The S3 route a tap opens.
    pub url: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub(crate) enum Update {
    /// Drop everything, then index `entries`.
    Replace {
        entries: Vec<Entry>,
    },
    Upsert {
        entries: Vec<Entry>,
    },
    Delete {
        ids: Vec<String>,
    },
    Clear,
}

/// RFC 3986 percent-encoding of everything but unreserved characters, the
/// inverse of the frontend's `decodeURIComponent`.
fn encode_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn collapsed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn first_text(blocks: &[tine_core::doc::DocBlock]) -> Option<String> {
    blocks.iter().find_map(|block| {
        let text = collapsed(block.visible_text());
        if text.is_empty() {
            first_text(&block.children)
        } else {
            Some(text)
        }
    })
}

/// The same walk over a loaded page's blocks (watcher updates): each body is
/// projected exactly as [`tine_core::doc::DocBlock::visible_text`] does.
fn first_dto_text(blocks: &[tine_core::model::BlockDto], is_org: bool) -> Option<String> {
    blocks.iter().find_map(|block| {
        let mut doc = tine_core::doc::DocBlock::new(block.raw.clone());
        doc.set_org(is_org);
        let text = collapsed(doc.visible_text());
        if text.is_empty() {
            first_dto_text(&block.children, is_org)
        } else {
            Some(text)
        }
    })
}

/// Whitespace-collapsed text cut to [`EXCERPT_CHARS`].
fn cut(text: String) -> String {
    if text.chars().count() <= EXCERPT_CHARS {
        return text;
    }
    let head: String = text.chars().take(EXCERPT_CHARS - 1).collect();
    format!("{}…", head.trim_end())
}

/// The first non-empty block's visible text, whitespace collapsed, cut to
/// [`EXCERPT_CHARS`].
pub(crate) fn excerpt(document: &tine_core::doc::Document) -> String {
    cut(first_text(&document.roots).unwrap_or_default())
}

fn entry_with(name: &str, excerpt: String) -> Entry {
    Entry {
        id: tine_core::refs::page_key(name),
        title: name.to_owned(),
        excerpt,
        url: format!("tine://page/{}", encode_component(name)),
    }
}

pub(crate) fn entry(name: &str, document: &tine_core::doc::Document) -> Entry {
    entry_with(name, excerpt(document))
}

/// [`entry`] for a page the store just loaded.
pub(crate) fn entry_for_page(page: &tine_core::model::PageDto) -> Entry {
    let is_org = page.format == tine_core::model::Format::Org;
    entry_with(
        &page.name,
        cut(first_dto_text(&page.blocks, is_org).unwrap_or_default()),
    )
}

fn root_of(slot: &crate::state::GraphSlot) -> String {
    slot.root_key.display().to_string()
}

/// Replace the index with `slot`'s pages. Called once the window's graph is
/// warm (graph.rs `warm_cache_async`).
pub(crate) fn reindex(app: &tauri::AppHandle, label: &str, slot: &crate::state::GraphSlot) {
    if !INDEXES || label != INDEXED_WINDOW {
        return;
    }
    let view = match slot.store.whole_graph() {
        Ok(view) => view,
        Err(error) => {
            crate::debug::diag_private("spotlight-reindex-failed", &format!("{error:?}"));
            return;
        }
    };
    let entries = view
        .corpus()
        .pages
        .iter()
        .map(|page| entry(&page.name, &page.document))
        .collect();
    *INDEXED_ROOT.lock().unwrap() = Some(root_of(slot));
    publish(app, &Update::Replace { entries });
}

/// Pages one publication changed: `(name, kind, removed)`.
pub(crate) type Changed = Vec<(String, PageKind, bool)>;

/// Bring the changed pages' items up to date (watcher.rs `dispatch`).
pub(crate) fn observe(
    app: &tauri::AppHandle,
    label: &str,
    slot: &std::sync::Arc<crate::state::GraphSlot>,
    changed: Changed,
) {
    if !INDEXES || label != INDEXED_WINDOW || changed.is_empty() {
        return;
    }
    if INDEXED_ROOT.lock().unwrap().as_deref() != Some(root_of(slot).as_str()) {
        return; // not indexed yet: the coming reindex covers this change
    }
    let (app, slot) = (app.clone(), slot.clone());
    // Page reads take the store writer: never on the dispatch thread.
    std::thread::spawn(move || {
        let mut entries = Vec::new();
        let mut ids = Vec::new();
        for (name, kind, removed) in changed {
            match (!removed)
                .then(|| slot.store.page_named(&name, kind))
                .transpose()
            {
                Ok(Some(Some(read))) => entries.push(entry_for_page(&read.doc)),
                Ok(_) => ids.push(tine_core::refs::page_key(&name)),
                Err(error) => {
                    crate::debug::diag_private("spotlight-page-read-failed", &format!("{error:?}"))
                }
            }
        }
        if !ids.is_empty() {
            publish(&app, &Update::Delete { ids });
        }
        if !entries.is_empty() {
            publish(&app, &Update::Upsert { entries });
        }
    });
}

/// The window's graph closed or switched: its pages leave the index.
pub(crate) fn clear(app: &tauri::AppHandle, label: &str) {
    if !INDEXES || label != INDEXED_WINDOW {
        return;
    }
    *INDEXED_ROOT.lock().unwrap() = None;
    publish(app, &Update::Clear);
}

/// A graph was removed from the known-graph list: clear the index if it was
/// the indexed one.
pub(crate) fn forget(app: &tauri::AppHandle, root: &str) {
    if !INDEXES {
        return;
    }
    let mut indexed = INDEXED_ROOT.lock().unwrap();
    if indexed.as_deref() == Some(root) {
        *indexed = None;
        drop(indexed);
        publish(app, &Update::Clear);
    }
}

// ---- Platform split: every shipped target is named. ----

/// iOS: Core Spotlight, through the native integrations plugin.
#[cfg(any(target_os = "ios"))]
const INDEXES: bool = true;
#[cfg(any(target_os = "ios"))]
fn publish(app: &tauri::AppHandle, update: &Update) {
    if let Err(error) = crate::native_integrations::spotlight(app, update) {
        crate::debug::diag_private("spotlight-update-failed", &error);
    }
}

/// No system search index Tine feeds on these targets.
#[cfg(any(
    target_os = "android",
    target_os = "linux",
    target_os = "windows",
    target_os = "macos"
))]
const INDEXES: bool = false;
#[cfg(any(
    target_os = "android",
    target_os = "linux",
    target_os = "windows",
    target_os = "macos"
))]
fn publish(_app: &tauri::AppHandle, _update: &Update) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(markdown: &str) -> tine_core::doc::Document {
        tine_core::doc::parse(markdown)
    }

    #[test]
    fn an_entry_is_title_excerpt_and_the_open_page_route() {
        let doc = document("title:: ignored\n\n- \n- First **real** line\n  continues\n- second\n");
        let entry = entry("Über Plan/2026", &doc);
        assert_eq!(entry.id, tine_core::refs::page_key("Über Plan/2026"));
        assert_eq!(entry.title, "Über Plan/2026");
        assert!(entry.excerpt.starts_with("First"), "{:?}", entry.excerpt);
        assert!(!entry.excerpt.contains('\n'));
        assert_eq!(entry.url, "tine://page/%C3%9Cber%20Plan%2F2026");
    }

    #[test]
    fn a_long_excerpt_is_cut_to_the_limit() {
        let doc = document(&format!("- {}\n", "word ".repeat(100)));
        let text = excerpt(&doc);
        assert_eq!(text.chars().count(), EXCERPT_CHARS);
        assert!(text.ends_with('…'));
        assert_eq!(excerpt(&document("")), "");
    }

    /// A watcher update must produce the same item as the warm reindex did,
    /// or an edit would flip a page's excerpt between two spellings.
    #[test]
    fn a_loaded_page_yields_the_same_entry_as_the_corpus() {
        let markdown = "- \n- TODO First [[real]] line\n  id:: 6512b1a4-0000-4000-8000-000000000001\n- second\n";
        let doc = document(markdown);
        let page = tine_core::projection::markdown_page_dto("Plan", "Plan", markdown);
        let from_corpus = entry("Plan", &doc);
        assert!(
            !from_corpus.excerpt.contains("id::"),
            "{:?}",
            from_corpus.excerpt
        );
        assert_eq!(
            serde_json::to_value(entry_for_page(&page)).unwrap(),
            serde_json::to_value(from_corpus).unwrap()
        );
    }

    #[test]
    fn updates_serialize_with_an_op_tag() {
        let json = serde_json::to_value(Update::Delete {
            ids: vec!["a".into()],
        })
        .unwrap();
        assert_eq!(json, serde_json::json!({"op": "delete", "ids": ["a"]}));
        let json = serde_json::to_value(Update::Clear).unwrap();
        assert_eq!(json, serde_json::json!({"op": "clear"}));
    }

    /// AGENTS.md section 2: a platform `cfg` list names every shipped target.
    #[test]
    fn every_shipped_target_is_named_exactly_once() {
        let source = include_str!("spotlight.rs");
        let production = source.split("#[cfg(test)]").next().unwrap();
        let mut named: Vec<&str> = production
            .split("#[cfg(any(")
            .skip(1)
            .step_by(2) // each arm has a const and a fn; count the const's list
            .flat_map(|arm| {
                arm.split("))]")
                    .next()
                    .unwrap()
                    .split('"')
                    .skip(1)
                    .step_by(2)
            })
            .collect();
        named.sort_unstable();
        assert_eq!(
            named,
            ["android", "ios", "linux", "macos", "windows"],
            "the Spotlight platform split must name Linux, Windows, macOS, iOS and Android \
             exactly once (AGENTS.md section 2; exemplar defender.rs)"
        );
    }
}
