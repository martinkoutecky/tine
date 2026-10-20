//! The configured daily template, found from the graph's files without the
//! graph-wide parse (BqbeQ, D-20261018-A14 GF4 follow-up).
//!
//! Graph-wide discovery (`WholeGraph::templates`) needs every page parsed: on a
//! cold launch of a 10k-page graph an empty today waited seconds for it before
//! its template could be applied. One configured name needs much less. Every
//! block that defines a template has a property key that folds to
//! `template`, so its file's bytes contain `template` (ASCII, any case): files
//! without it cannot define one and are not parsed. The files that do are
//! parsed with the same parser and judged by the same per-block answer
//! (`tine_core::template::template_of`) as the graph-wide listing.
//!
//! The answer is settled only where it is the one graph-wide discovery gives:
//! no block, or exactly one block, defines the name. With two or more, the
//! graph's page order picks the first and that order is not visible from the
//! files, so the caller waits for the graph as before. A listing or read
//! failure, or a candidate in a duplicate-day journal (a stray the graph
//! leaves out of its pages), is also unsettled.
//!
//! Cost: one directory walk of the graph (O(P) entries), one journals-area walk,
//! one read of every graph-text file (O(graph bytes)), and a parse of each file
//! whose bytes contain `template`; one page read for the winner's page name.
//! No write, no lock held across files, no persisted state.

use std::collections::{HashMap, HashSet};
use std::io;

use tine_core::model::TemplateDto;
use tine_store::{Area, FileId, Store, StoreError};

/// What the files say about one template name.
#[derive(Debug)]
pub enum NamedTemplate {
    /// Graph-wide discovery gives this answer: the one template so named, or
    /// none.
    Settled(Option<TemplateDto>),
    /// Only graph-wide discovery can answer (two or more definitions, a file
    /// that could not be listed or read, or `stop` was raised).
    Unsettled,
}

/// Find the template named exactly `name` (the configured
/// `:default-templates {:journals …}` value; case and inner whitespace are
/// significant, as in graph-wide discovery) from the graph's files. `stop` is
/// polled between files; once it returns true the answer is `Unsettled`.
/// A closed store is an error.
pub fn named_template_from_files(
    store: &Store,
    name: &str,
    stop: impl Fn() -> bool,
) -> io::Result<NamedTemplate> {
    if stop() {
        return Ok(NamedTemplate::Unsettled);
    }
    let listing = store
        .scan_area(Area::Graph, None)
        .map_err(crate::store_error)?;
    if !listing.unreadable.is_empty() {
        return Ok(NamedTemplate::Unsettled);
    }
    let journals = store
        .scan_area(Area::Journals, None)
        .map_err(crate::store_error)?;
    if !journals.unreadable.is_empty() {
        return Ok(NamedTemplate::Unsettled);
    }
    let mut per_day: HashMap<i64, usize> = HashMap::new();
    for entry in journals.files.iter().filter(|e| e.page.is_some()) {
        if let Some(day) = entry.day {
            *per_day.entry(day.0).or_default() += 1;
        }
    }
    let shared_day: HashSet<FileId> = journals
        .files
        .iter()
        .filter(|e| e.page.is_some() && e.day.is_some_and(|day| per_day[&day.0] > 1))
        .map(|e| e.id.clone())
        .collect();
    let mut found: Vec<(tine_store::PageId, Vec<tine_core::model::BlockDto>)> = Vec::new();
    for entry in &listing.files {
        let Some(page) = entry.page.as_ref() else {
            continue;
        };
        if !tine_store::is_graph_text(&entry.id) {
            continue;
        }
        if stop() {
            return Ok(NamedTemplate::Unsettled);
        }
        let bytes = match store.read(&entry.id, Some(tine_store::PARSE_INPUT_MAX_BYTES)) {
            Ok((bytes, _)) => bytes,
            // Deleted since the listing: the graph will not have it either.
            Err(StoreError::NotFound) => continue,
            Err(StoreError::Closed) => return Err(crate::store_error(StoreError::Closed)),
            // Also a file over the parse cap (TooLarge, review F4): the graph answers.
            Err(_) => return Ok(NamedTemplate::Unsettled),
        };
        if !tine_core::template::may_define_template(&bytes) {
            continue;
        }
        let text = match crate::parsed_text::read(store, &entry.id) {
            Ok((text, _)) => text,
            Err(_) => return Ok(NamedTemplate::Unsettled),
        };
        let doc = crate::conflicts::parse(&text, crate::conflicts::format(&entry.id));
        let mut here = Vec::new();
        tine_core::template::templates_in_document(&doc, &mut |found_name, blocks| {
            if found_name == name {
                here.push(blocks);
            }
        });
        if here.is_empty() {
            continue;
        }
        if shared_day.contains(&entry.id) {
            return Ok(NamedTemplate::Unsettled);
        }
        found.extend(here.into_iter().map(|blocks| (page.clone(), blocks)));
        if found.len() > 1 {
            return Ok(NamedTemplate::Unsettled);
        }
    }
    let Some((page, blocks)) = found.pop() else {
        return Ok(NamedTemplate::Settled(None));
    };
    // The page's display name and kind, as graph-wide discovery reports them.
    let read = match store.page(&page) {
        Ok(read) => read,
        Err(StoreError::Closed) => return Err(crate::store_error(StoreError::Closed)),
        Err(_) => return Ok(NamedTemplate::Unsettled),
    };
    Ok(NamedTemplate::Settled(Some(TemplateDto {
        name: name.to_owned(),
        blocks,
        page: read.doc.name,
        kind: read.doc.kind,
    })))
}

#[cfg(test)]
#[path = "templates_tests.rs"]
mod templates_tests;
