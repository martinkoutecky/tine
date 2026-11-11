//! The text a page rename writes: every place the one reference answer
//! (`reference_evidence`, the walker over the lsdoc parse of each block) says
//! names a renamed page, plus the page-name arguments of queries and macros,
//! respelled for the new name. Nothing else changes (conceptual model step 2,
//! batch 6; design note `cm-step2-rename-scanner-design.md`).
//!
//! **Classes.** A *reference* (link, tag, embed, nested-link name, Org labelled
//! link, Org headline tag, linkable property member, property key) is
//! rewritten; task markers and priorities are references that are never
//! rewritten (D-20261019-M2(4)). Bare `alias::`/`aliases::` members follow
//! too (D-20261105-CM13, a deliberate OG difference). A *rewrite target*
//! is not a reference but names a page: a
//! query's page-name arguments (`[[x]]`, `#x`, `(page x)`, `(page-ref x)`,
//! `(namespace x)`, `(page-tags …)` and a TQL page-name literal; D-20261031-CMQ),
//! a `[[x]]` in the argument of any other macro but `embed` (cloze, renderer,
//! plugin macros; manager ruling Q2), and the stem of an Org `file:` link
//! (`reference_evidence::rename_targets`). Plain query search text is neither.
//! A query macro inside code stays as written, whether the code is in the
//! block's text, a fence, or a property's value (`query_edits`).
//!
//! **Intentional OG differences** (manager ruling Q2/Q3/Q4, 2026-11-03; a veto
//! item for Martin). OG's `replace-page-ref!` replaces a literal `[[old]]`
//! anywhere in a referencing block's text, and `replace-tag-ref!` a regex
//! `#old`. Here only the parser's references are rewritten, so a `[[old]]`
//! inside math, an HTML comment, inline HTML, after a `\[[` escape, in a
//! `:LOGBOOK:` drawer or in code stays as written, while `word#old`, an Org
//! `[[old][label]]`, Org headline tags and an Org drawer `:tags:` value (which
//! OG's literal replace misses) are rewritten.
//!
//! **Offsets.** Each block's parser offsets are in its raw text (the header line
//! after its structural prefix, continuation lines dedented); they are mapped
//! back to the page text line by line, and an LF offset back to the file's own
//! bytes past each CRLF, so every byte that is not a rewritten name is kept.
//!
//! Cost: one lsdoc parse per block (the projection the store builds anyway),
//! a second one for a block holding `{{` or an Org `[[file:`, plus one query
//! parse per query macro and one read of each property value that holds one
//! (`PropertyValueCode`); O(page bytes). Pure.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::ops::Range;

use crate::config::FileNameFormat;
use crate::doc::{property_key_norm, DocBlock};
use crate::outline::{self, OutlineFormat};
use crate::query::macro_text::{
    macro_safe, query_macro_extents, recognizable_macro, split_trailing_map, FormFamily,
};
use crate::reference_evidence::RewriteTarget;
use crate::refs::{is_linkable_property_separator, page_key};
use crate::text_trace::{TextTrace, TraceError};

/// A covered site the span writer cannot faithfully realize. B3 decides
/// admission; this writer reports the evidence without refusing the site.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Site {
    pub range: Range<usize>,
    pub reason: Unrealized,
}

/// Why a covered substitution did not have a faithful native spelling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unrealized {
    HeadlineTag,
    PropertyKey,
    StoredQuery,
    CrossPiece,
    MacroLink,
}

/// One physical file's span rewrite and complete-text evidence, all ephemeral.
#[derive(Clone, Debug)]
pub struct RenameProposal {
    pub text: String,
    pub trace: TextTrace,
    pub unrealized: Vec<Site>,
    pub rewritten: HashSet<(String, String)>,
}

impl RenameProposal {
    /// Rebind the own title through the reader's title-name span. All its key,
    /// spacing and terminators remain protected, just as for reference edits.
    pub fn rebind_title(&mut self, org: bool, name: &str) -> Result<(), TraceError> {
        let original = &self.trace.inputs[0];
        let format = if org {
            crate::model::Format::Org
        } else {
            crate::model::Format::Md
        };
        let Some(range) = crate::model::title_name_span(original, format) else {
            return Ok(());
        };
        let wrappers: HashSet<_> = self
            .trace
            .wrappers
            .iter()
            .map(|id| self.trace.pieces[*id].range.clone())
            .collect();
        let mut edits: Vec<_> = self
            .trace
            .pieces
            .iter()
            .enumerate()
            .filter_map(|(id, p)| {
                self.trace
                    .substitution
                    .get(&id)
                    .map(|s| (p.range.clone(), s.clone()))
            })
            .collect();
        let at = edits.partition_point(|(r, _)| r.start < range.start);
        edits.insert(at, (range, name.into()));
        let mut trace = TextTrace::default();
        trace.add_input(original, &edits, &[])?;
        trace.wrappers = trace
            .pieces
            .iter()
            .enumerate()
            .filter(|(_, p)| wrappers.contains(&p.range))
            .map(|(id, _)| id)
            .collect();
        trace.ordered_output();
        self.text = trace.render()?;
        trace.check_shadow(&self.text)?;
        self.trace = trace;
        Ok(())
    }
}

/// One rename, prepared once per text: old page key to new display name, and
/// the property-key fold of each old name (a property key references the page
/// whose name folds to it, `reference_evidence::visit_reference_targets`).
pub(crate) struct Renames<'a> {
    pages: &'a HashMap<String, String>,
    /// A query's page-name arguments that name one of these keys take this
    /// name instead of `pages`' (a merge through an alias: the survivor's own
    /// name, ruling (a) of D-20261103-CMA, the model's `queryMap`); every
    /// other query argument follows `pages`.
    queries: Option<&'a HashMap<String, String>>,
    keys: HashMap<String, &'a str>,
    file_name_format: FileNameFormat,
    record: bool,
    unrealized: std::cell::RefCell<Vec<Site>>,
}

impl<'a> Renames<'a> {
    /// `pages` maps `page_key(old)` to the new display name; `queries` (keyed
    /// the same way) overrides it for query page-name arguments only.
    pub(crate) fn new(
        pages: &'a HashMap<String, String>,
        queries: Option<&'a HashMap<String, String>>,
        file_name_format: FileNameFormat,
        record: bool,
    ) -> Self {
        let keys = pages
            .iter()
            .map(|(old, to)| (property_key_norm(old), to.as_str()))
            .collect();
        Self {
            pages,
            queries: queries.filter(|queries| !queries.is_empty()),
            keys,
            file_name_format,
            record,
            unrealized: Default::default(),
        }
    }

    fn page(&self, name: &str) -> Option<Cow<'a, str>> {
        crate::refs::renamed_page_name(name, self.pages)
    }

    /// The new name of a query's page-name argument.
    fn query_page(&self, name: &str) -> Option<Cow<'a, str>> {
        self.queries
            .and_then(|queries| queries.get(&page_key(name)))
            .map(|to| Cow::Borrowed(to.as_str()))
            .or_else(|| self.page(name))
    }

    fn edit(&self, range: Range<usize>, name: &str, to: &str, spelling: String) -> Edit {
        (
            range,
            spelling,
            self.record
                .then(|| Box::new((name.to_owned(), to.to_owned()))),
        )
    }

    fn miss(&self, range: Range<usize>, reason: Unrealized) {
        self.unrealized.borrow_mut().push(Site { range, reason });
    }
}

type Edit = (Range<usize>, String, Option<Box<(String, String)>>);

/// Rewrite a whole page `text` (`.md` or `.org`). Returns `text` unchanged when
/// nothing names a renamed page.
pub(crate) fn rewrite_page(
    text: &str,
    is_org: bool,
    renames: &Renames<'_>,
    rewritten: Option<&mut HashSet<(String, String)>>,
) -> String {
    if renames.pages.is_empty() {
        return text.to_owned();
    }
    // The page parse reads LF text (`doc::normalize_line_endings`): find the
    // edits there and place them in the file's own bytes. Normalizing drops the
    // `\r` of each CRLF and turns a lone `\r` into `\n`, so an LF offset moves
    // by the CRLFs before it.
    let lf = crate::doc::normalize_line_endings(text);
    let edits = page_edits(&lf, is_org, renames);
    if edits.is_empty() {
        return text.to_owned();
    }
    let crlf = lf_offsets_of_crlf(text);
    let original = |at: usize| at + crlf.partition_point(|lf_at| *lf_at < at);
    splice(
        text,
        edits
            .into_iter()
            .map(|(range, to, name)| (original(range.start)..original(range.end), to, name)),
        rewritten,
    )
}

/// The same legal span finder as `rewrite_page`, carrying its partition and
/// omitted covered sites. No additional block walk or I/O.
pub(crate) fn proposal(
    text: &str,
    is_org: bool,
    renames: &Renames<'_>,
) -> Result<RenameProposal, TraceError> {
    let lf = crate::doc::normalize_line_endings(text);
    let edits = if renames.pages.is_empty() && renames.queries.is_none() {
        Vec::new()
    } else {
        page_edits(&lf, is_org, renames)
    };
    let crlf = lf_offsets_of_crlf(text);
    let original = |at: usize| at + crlf.partition_point(|lf_at| *lf_at < at);
    let mut rewritten = HashSet::new();
    let mut wrappers = HashSet::new();
    let edits: Vec<_> = edits
        .into_iter()
        .map(|(range, to, name)| {
            let range = original(range.start)..original(range.end);
            if text[range.clone()] != to {
                if let Some(name) = name {
                    rewritten.insert(*name);
                }
            }
            // Encoders return native wrappers; the substitution itself names
            // only the semantic token. Keep their inserts explicit in evidence.
            let to = if let Some(inner) = to.strip_prefix("[[").and_then(|s| s.strip_suffix("]]")) {
                if text[range.clone()] != *inner {
                    wrappers.insert(range.clone());
                    inner.to_owned()
                } else {
                    to
                }
            } else {
                to
            };
            (range, to)
        })
        .collect();
    let mut trace = TextTrace::default();
    trace.add_input(text, &edits, &[])?;
    trace.wrappers = trace
        .pieces
        .iter()
        .enumerate()
        .filter(|(_, p)| wrappers.contains(&p.range))
        .map(|(id, _)| id)
        .collect();
    trace.ordered_output();
    let text = trace.render()?;
    trace.check_shadow(&text)?;
    let unrealized = renames
        .unrealized
        .take()
        .into_iter()
        .map(|site| Site {
            range: original(site.range.start)..original(site.range.end),
            ..site
        })
        .collect();
    Ok(RenameProposal {
        text,
        trace,
        unrealized,
        rewritten,
    })
}

/// Rewrite one held block's raw text for rename inspection.
pub(crate) fn rewrite_block(
    raw: &str,
    is_org: bool,
    renames: &Renames<'_>,
    rewritten: Option<&mut HashSet<(String, String)>>,
) -> String {
    let mut block = DocBlock::new(raw);
    block.set_org(is_org);
    splice(
        raw,
        settle(block_edits(raw, is_org, &block, renames)),
        rewritten,
    )
}

/// Rewrite a held page's pre-block text for rename inspection.
pub(crate) fn rewrite_pre_block(
    pre: &str,
    is_org: bool,
    renames: &Renames<'_>,
    rewritten: Option<&mut HashSet<(String, String)>>,
) -> String {
    splice(
        pre,
        settle(pre_block_edits(pre, is_org, renames)),
        rewritten,
    )
}

/// The LF-text offset of the `\n` of every CRLF in `text`.
fn lf_offsets_of_crlf(text: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for (removed, (at, _)) in text.match_indices("\r\n").enumerate() {
        out.push(at - removed);
    }
    out
}

/// Apply source-ordered, non-overlapping edits.
fn splice(
    text: &str,
    edits: impl DoubleEndedIterator<Item = Edit>,
    mut rewritten: Option<&mut HashSet<(String, String)>>,
) -> String {
    let mut out = text.to_owned();
    for (range, to, name) in edits.rev() {
        if let (Some(names), Some(name)) = (&mut rewritten, name) {
            if text[range.clone()] != to {
                names.insert(*name);
            }
        }
        out.replace_range(range, &to);
    }
    out
}

/// Source order, one edit per place: an edit inside an earlier, wider one is
/// dropped (the outer node is respelled whole, as `#[[x]]` is by its tag).
fn settle(mut edits: Vec<Edit>) -> std::vec::IntoIter<Edit> {
    edits.sort_by(|a, b| {
        a.0.start
            .cmp(&b.0.start)
            .then_with(|| b.0.end.cmp(&a.0.end))
    });
    let mut out: Vec<Edit> = Vec::with_capacity(edits.len());
    for edit in edits {
        if out.last().is_some_and(|last| edit.0.start < last.0.end) {
            continue;
        }
        out.push(edit);
    }
    out.into_iter()
}

/// The edits of LF page `text`, in page offsets, settled.
fn page_edits(text: &str, is_org: bool, renames: &Renames<'_>) -> Vec<Edit> {
    let format = if is_org {
        OutlineFormat::Org
    } else {
        OutlineFormat::Markdown
    };
    // An outline lsdoc cannot represent has no blocks: the page parse reads the
    // whole text as the pre-block (`doc::parse`), and so does the rename.
    let headers = outline::headers(text, format).unwrap_or_default();
    let body = text.strip_suffix('\n').unwrap_or(text);
    let lines: Vec<&str> = if body.is_empty() {
        Vec::new()
    } else {
        body.split('\n').collect()
    };
    let mut line_starts = vec![0usize];
    line_starts.extend(text.match_indices('\n').map(|(at, _)| at + 1));
    let mut edits = Vec::new();
    let first = headers.first().map_or(lines.len(), |header| header.line);
    if first > 0 {
        let pre_end = line_starts.get(first).copied().unwrap_or(text.len());
        edits.extend(pre_block_edits(&text[..pre_end], is_org, renames));
    }
    for (n, header) in headers.iter().enumerate() {
        let end = headers.get(n + 1).map_or(lines.len(), |next| next.line);
        // Mirror `outline::blocks`: the header line after its prefix, then each
        // continuation dedented; remember where each piece starts in the page.
        let first_line = &lines[header.line][header.prefix_len..];
        let mut raw = first_line.to_string();
        let mut pieces = vec![(0usize, line_starts[header.line] + header.prefix_len)];
        for (k, line) in lines.iter().enumerate().take(end).skip(header.line + 1) {
            let kept = outline::strip_layout_ws(line, header.content_indent);
            raw.push('\n');
            pieces.push((raw.len(), line_starts[k] + line.len() - kept.len()));
            raw.push_str(kept);
        }
        let mut block = DocBlock::new(raw.as_str());
        block.set_org(is_org);
        let misses = renames.unrealized.borrow().len();
        for (range, to, name) in block_edits(&raw, is_org, &block, renames) {
            let at = pieces.partition_point(|(start, _)| *start <= range.start) - 1;
            let (piece_start, page_start) = pieces[at];
            let piece_end = pieces.get(at + 1).map_or(raw.len(), |(start, _)| start - 1);
            if range.end <= piece_end {
                let shift = page_start - piece_start;
                edits.push((range.start + shift..range.end + shift, to, name));
            } else {
                renames.miss(range, Unrealized::CrossPiece);
            }
        }
        for site in &mut renames.unrealized.borrow_mut()[misses..] {
            let mapped = |at| {
                let (start, page) = pieces[pieces.partition_point(|(start, _)| *start <= at) - 1];
                page + at - start
            };
            site.range = mapped(site.range.start)..mapped(site.range.end);
        }
    }
    settle(edits).collect()
}

/// The edits of a pre-block, in its own offsets: its page properties' and its
/// prose's (the two blocks the store reads there, batch 8), unsettled.
fn pre_block_edits(pre: &str, is_org: bool, renames: &Renames<'_>) -> Vec<Edit> {
    let mut edits = property_edits(pre, is_org, renames);
    if let Some(range) = crate::page_properties::preamble_content_range(pre, is_org) {
        let raw = &pre[range.clone()];
        let mut block = DocBlock::new(raw);
        block.set_org(is_org);
        let misses = renames.unrealized.borrow().len();
        edits.extend(
            block_edits(raw, is_org, &block, renames)
                .into_iter()
                .map(|(r, to, name)| (r.start + range.start..r.end + range.start, to, name)),
        );
        for site in &mut renames.unrealized.borrow_mut()[misses..] {
            site.range = site.range.start + range.start..site.range.end + range.start;
        }
    }
    edits
}

/// The edits of a pre-block's page properties, in its own offsets. The store
/// reads them as a synthesized block (`page_properties::page_property_raw`);
/// its sites map back to each property's key and value in `pre`.
fn property_edits(pre: &str, is_org: bool, renames: &Renames<'_>) -> Vec<Edit> {
    let raw = crate::page_properties::page_property_raw(pre, is_org);
    if raw.is_empty() {
        return Vec::new();
    }
    let regions = crate::block_regions::parse_document(pre, is_org);
    // (synthesized start, synthesized end, pre start) for each key and value.
    let mut pieces: Vec<(usize, usize, usize)> = Vec::new();
    let mut line_start = if is_org { ":PROPERTIES:\n".len() } else { 0 };
    for property in regions.page_properties() {
        let key = if is_org {
            property.key.to_ascii_lowercase()
        } else {
            property.key.clone()
        };
        let (key_at, value_at) = if is_org {
            (line_start + 1, line_start + key.len() + 3)
        } else {
            (line_start, line_start + key.len() + 3)
        };
        line_start = value_at + property.value.len() + 1;
        if raw.get(key_at..key_at + key.len()) != Some(key.as_str())
            || raw.get(value_at..value_at + property.value.len()) != Some(property.value.as_str())
        {
            // Not the synthesized layout this mapping assumes: no edits at all
            // rather than an edit in the wrong place.
            return Vec::new();
        }
        if property.key_range.slice(pre).len() == key.len() {
            pieces.push((key_at, key_at + key.len(), property.key_range.0));
        }
        let authored = property.value_range.slice(pre);
        if let Some(offset) = authored.find(property.value.as_str()) {
            pieces.push((
                value_at,
                value_at + property.value.len(),
                property.value_range.0 + offset,
            ));
        }
    }
    let block = crate::page_properties::property_projection(&raw, is_org);
    let misses = renames.unrealized.borrow().len();
    let edits = block_edits(&raw, is_org, &block, renames);
    for site in &mut renames.unrealized.borrow_mut()[misses..] {
        if let Some((start, _, at)) = pieces
            .iter()
            .find(|(start, end, _)| *start <= site.range.start && site.range.end <= *end)
        {
            site.range = site.range.start - start + at..site.range.end - start + at;
        } else {
            site.reason = Unrealized::CrossPiece;
            site.range = 0..pre.len();
        }
    }
    edits
        .into_iter()
        .filter_map(|(range, to, name)| {
            let Some((start, _, at)) = pieces
                .iter()
                .find(|(start, end, _)| *start <= range.start && range.end <= *end)
            else {
                renames.miss(0..pre.len(), Unrealized::CrossPiece);
                return None;
            };
            Some((range.start - start + at..range.end - start + at, to, name))
        })
        .collect()
}

/// The edits of one block, in its raw offsets (unsettled).
fn block_edits(raw: &str, is_org: bool, block: &DocBlock, renames: &Renames<'_>) -> Vec<Edit> {
    #[cfg(test)]
    BLOCK_WALKS.with(|walks| walks.set(walks.get() + 1));
    let projection = block.projection();
    let source = projection.reference_source();
    let mut edits = Vec::new();
    for reference in source.explicit {
        let range = reference.range.clone();
        let Some(node) = raw.get(range.clone()) else {
            continue;
        };
        match reference.rule {
            // Permanent exceptions (D-20261019-M2(4)).
            "explicit_task_marker" | "explicit_priority" => {}
            // OG `replace-property-ref!`: the new name lower-cased, whitespace
            // runs hyphenated (D-20261019-M2(4), manager ruling Q1).
            "explicit_property_key" => {
                // The key names its page by page identity (`Über::` is page
                // "über"); `keys` is keyed by the old name's key spelling.
                if let Some(to) = renames
                    .keys
                    .get(&property_key_norm(&page_key(&reference.name)))
                {
                    if let Some(key) = property_key_spelling(to) {
                        if key != *to {
                            renames.miss(range.clone(), Unrealized::PropertyKey);
                        }
                        edits.push(renames.edit(range, &reference.name, to, key));
                    } else {
                        renames.miss(range, Unrealized::PropertyKey);
                    }
                }
            }
            "implicit_linkable_property" => {
                if let Some(to) = renames.page(&reference.name) {
                    edits.push(renames.edit(range, &reference.name, &to, member_spelling(&to)));
                }
            }
            "explicit_tag" => {
                let Some(to) = renames.page(&reference.name) else {
                    continue;
                };
                if node.starts_with('#') {
                    let (start, end, spelling) = if node.starts_with("#[[") && node.ends_with("]]")
                    {
                        (3, node.len() - 2, to.to_string())
                    } else {
                        (1, node.len(), tag_for(&to)[1..].to_owned())
                    };
                    edits.push(renames.edit(
                        range.start + start..range.start + end,
                        &reference.name,
                        &to,
                        spelling,
                    ));
                } else if headline_tag_ok(&to) {
                    // An Org headline tag `:name:`: the range is the name.
                    edits.push(renames.edit(range, &reference.name, &to, to.to_string()));
                } else {
                    renames.miss(range, Unrealized::HeadlineTag);
                }
            }
            "explicit_link" => link_edits(raw, range, &reference.name, renames, &mut edits),
            "explicit_nested_link" => nested_edits(node, range.start, renames, &mut edits),
            "explicit_embed" => {
                if let (Some(open), Some(close)) = (node.find("[["), node.rfind("]]")) {
                    if open + 2 <= close {
                        if let Some(to) = renames.page(&node[open + 2..close]) {
                            edits.push(renames.edit(
                                range.start + open + 2..range.start + close,
                                &node[open + 2..close],
                                &to,
                                to.to_string(),
                            ));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    edits.extend(query_edits(raw, &projection.regions, is_org, renames));
    // Rewrite targets need the parse again; only a block that can hold one
    // pays for it.
    if raw.contains("{{") || (is_org && raw.contains("[[file:")) {
        for target in crate::reference_evidence::rename_targets(raw, is_org) {
            match target {
                RewriteTarget::FileLink(range) => {
                    let inner = range.start + 2..range.end.saturating_sub(2);
                    if let Some((stem, to, name)) = raw
                        .get(inner.clone())
                        .and_then(|inner| file_link_stem_edit(inner, renames))
                    {
                        edits.push((inner.start + stem.start..inner.start + stem.end, to, name));
                    }
                }
                // A new name with `}` would end the macro early: the macro keeps
                // its old bytes, as a query macro does.
                RewriteTarget::MacroLink(reference) => {
                    let mut local = Vec::new();
                    let node = &raw[reference.range.clone()];
                    if reference.rule == "explicit_nested_link" {
                        nested_edits(node, reference.range.start, renames, &mut local);
                    } else {
                        link_edits(raw, reference.range, &reference.name, renames, &mut local);
                    }
                    for edit in local {
                        if edit.1.contains('}') {
                            renames.miss(edit.0, Unrealized::MacroLink);
                        } else {
                            edits.push(edit);
                        }
                    }
                }
            }
        }
    }
    edits
}

/// The names inside a nested link `[[a [[x]] c]]` that a rename respells.
fn nested_edits(node: &str, at: usize, renames: &Renames<'_>, edits: &mut Vec<Edit>) {
    for inner in crate::block_regions::nested_reference_ranges(node) {
        if let Some(to) = renames.page(&node[inner.clone()]) {
            edits.push(renames.edit(
                at + inner.start..at + inner.end,
                &node[inner],
                &to,
                to.to_string(),
            ));
        }
    }
}

/// A link node `[[x]]`, Org `[[x][label]]`, Markdown `[label]([[x]])` or an
/// Org `[[file:…][label]]` (named by its label): respell the target name.
fn link_edits(
    raw: &str,
    range: Range<usize>,
    name: &str,
    renames: &Renames<'_>,
    edits: &mut Vec<Edit>,
) {
    let Some(to) = renames.page(name) else {
        return;
    };
    let node = &raw[range.clone()];
    let mut push = |inner: Range<usize>| {
        if renames.page(&node[inner.clone()]).as_deref() == Some(to.as_ref()) {
            edits.push(renames.edit(
                range.start + inner.start..range.start + inner.end,
                name,
                &to,
                to.to_string(),
            ));
        }
    };
    if let Some(body) = node.strip_prefix("[[").and_then(|b| b.strip_suffix("]]")) {
        match body.find("][") {
            // Org file link: its label names the page (the evidence's rule);
            // the path stem is a rewrite target of its own.
            Some(split) if body.starts_with("file:") => push(2 + split + 2..node.len() - 2),
            Some(split) => push(2..2 + split),
            None => push(2..node.len() - 2),
        }
    } else if let Some(split) = node.rfind("](") {
        let target = &node[split + 2..node.len().saturating_sub(1)];
        let lead = target.len() - target.trim_start().len();
        let trimmed = target.trim();
        if trimmed.starts_with("[[") && trimmed.ends_with("]]") && trimmed.len() >= 4 {
            let start = split + 2 + lead + 2;
            push(start..start + trimmed.len() - 4);
        }
    }
}

/// Where the code is in the property values of one block, each value read once.
///
/// The outline parser reads a property line as one `Properties` node and keeps
/// each value as text, which is why the block's literal regions end at the
/// node; the app shows the value as inline text (`PagePropertyValue`), so the
/// value parsed on its own, as the parser would read it as text, says what is
/// code there (a backtick span, an Org `=…=` or `~…~` span).
///
/// Lower bound: a value holding a query macro is read once, however many
/// macros it holds (O(its bytes)), and a value without one is not read; each
/// macro is then a binary search of that read's code ranges. `properties` are
/// in source order and disjoint (`block_regions`' visitors append in source
/// order), and so are a parse's `literals` (sorted and merged).
struct PropertyValueCode<'a> {
    raw: &'a str,
    regions: &'a crate::block_regions::BlockRegions,
    is_org: bool,
    /// The code ranges of each value already read, in the value's own offsets,
    /// by its index in `regions.properties`.
    read: HashMap<usize, Vec<crate::block_regions::Range>>,
}

impl<'a> PropertyValueCode<'a> {
    fn new(raw: &'a str, regions: &'a crate::block_regions::BlockRegions, is_org: bool) -> Self {
        Self {
            raw,
            regions,
            is_org,
            read: HashMap::new(),
        }
    }

    /// Whether `at` (an offset in `raw`) is inside code written in a property's
    /// value.
    fn contains(&mut self, at: usize) -> bool {
        let (raw, is_org, properties) = (self.raw, self.is_org, &self.regions.properties);
        let index = properties.partition_point(|property| property.value_range.1 <= at);
        let Some(property) = properties
            .get(index)
            .filter(|property| property.value_range.contains(at))
        else {
            return false;
        };
        let code = self.read.entry(index).or_insert_with(|| {
            #[cfg(test)]
            VALUE_PARSES.with(|reads| reads.set(reads.get() + 1));
            crate::block_regions::parse(property.value_range.slice(raw), is_org).literals
        });
        let at = at - property.value_range.0;
        let next = code.partition_point(|range| range.1 <= at);
        code.get(next).is_some_and(|range| range.0 <= at)
    }
}

#[cfg(test)]
thread_local! {
    /// Property values read for code on this thread (tests reset it first).
    static VALUE_PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static BLOCK_WALKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The page-name arguments of each query macro in `raw` (D-20261031-CMQ):
/// what the OG DSL parse reads as a page name, and each TQL page-name literal.
/// A macro whose rewritten argument would not read back as the same macro (a
/// new name with a `}`) keeps its old bytes, and one inside parser-owned
/// literal text (code, in the text or in a property's value) is no macro.
fn query_edits(
    raw: &str,
    regions: &crate::block_regions::BlockRegions,
    is_org: bool,
    renames: &Renames<'_>,
) -> Vec<Edit> {
    let mut edits = Vec::new();
    if !raw.contains("{{") {
        return edits;
    }
    let mut value_code = PropertyValueCode::new(raw, regions, is_org);
    for extent in query_macro_extents(raw) {
        if regions.literal_at(extent.start) || value_code.contains(extent.start) {
            continue;
        }
        let family = FormFamily::for_macro_name(&extent.name);
        let argument_end = extent.end - 2;
        let argument_start = argument_end - extent.argument.len();
        let (form, _) = split_trailing_map(&extent.argument, family);
        let form_start =
            argument_start + extent.argument.len() - extent.argument.trim_start().len();
        if raw.get(argument_start..argument_end) != Some(extent.argument.as_str())
            || !raw[form_start..].starts_with(form.as_str())
            || !crate::query::query_source_within_limit(&form)
            || !crate::query::query_nesting_within_limit(&form)
        {
            continue;
        }
        let names = match family {
            FormFamily::Tql => crate::query::tql::page_name_literals(&form),
            FormFamily::Edn if crate::query::is_advanced(&form) => Vec::new(),
            FormFamily::Edn => crate::query::og::page_name_arguments(&form),
        };
        let mut local: Vec<Edit> = Vec::new();
        for (range, name) in names {
            let Some(to) = renames.query_page(&name) else {
                continue;
            };
            let token = &form[range.clone()];
            let spelled = match family {
                FormFamily::Tql => crate::query::print::sql_string(&to),
                FormFamily::Edn => og_token_spelling(token, &to),
            };
            let (range, spelled) = if let Some(at) = token.find(&name).filter(|_| !name.is_empty())
            {
                let prefix = &token[..at];
                let suffix = &token[at + name.len()..];
                if spelled == format!("{prefix}{to}{suffix}") {
                    (
                        range.start + at..range.start + at + name.len(),
                        to.to_string(),
                    )
                } else {
                    (range, spelled)
                }
            } else {
                (range, spelled)
            };
            local.push(renames.edit(
                form_start + range.start..form_start + range.end,
                &name,
                &to,
                spelled,
            ));
        }
        if local.is_empty() {
            continue;
        }
        let mut argument = extent.argument.clone();
        for (range, to, _) in local.iter().rev() {
            argument.replace_range(range.start - argument_start..range.end - argument_start, to);
        }
        if macro_safe(&argument, family).is_ok()
            && recognizable_macro(&extent.name, &argument).is_ok()
        {
            edits.extend(local);
        } else {
            for (range, _, _) in local {
                renames.miss(range, Unrealized::StoredQuery);
            }
        }
    }
    edits
}

/// An OG DSL page-name token respelled for `to`, keeping its form.
fn og_token_spelling(token: &str, to: &str) -> String {
    use crate::query::print::{quote_str, word};
    if token.starts_with("\"[[") {
        quote_str(&format!("[[{to}]]"))
    } else if token.starts_with("[[") {
        format!("[[{to}]]")
    } else if token.starts_with('#') {
        tag_for(to)
    } else if token.starts_with('"') || to.starts_with(['#', ':', '[', '"']) || to.contains(',') {
        quote_str(to)
    } else {
        word(to)
    }
}

/// `#to` if `to` is a bare-tag-safe name, else `#[[to]]`.
pub(crate) fn tag_for(to: &str) -> String {
    let bare = |c: char| c.is_alphanumeric() || matches!(c, '-' | '_' | '/' | '.');
    if !to.is_empty() && to.chars().all(bare) {
        format!("#{to}")
    } else {
        format!("#[[{to}]]")
    }
}

/// A linkable property member: bare when it stays one member, else `[[to]]`.
pub(crate) fn member_spelling(to: &str) -> String {
    if to.contains(is_linkable_property_separator)
        || to.starts_with(['#', '"'])
        || to.contains("[[")
        || to.contains("]]")
    {
        format!("[[{to}]]")
    } else {
        to.to_owned()
    }
}

/// OG `replace-property-ref!`'s key: lower-cased, whitespace runs hyphenated.
/// `None` when that is no property key (empty, or holding a `:`).
fn property_key_spelling(to: &str) -> Option<String> {
    let key = to
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-");
    (!key.is_empty() && !key.contains(':')).then_some(key)
}

/// An Org headline tag holds no whitespace and no `:`; a name that does
/// cannot be written there, so that tag keeps the old name.
fn headline_tag_ok(to: &str) -> bool {
    !to.is_empty() && !to.contains(':') && !to.contains(char::is_whitespace)
}

/// Respell an Org `file:` link path whose stem names a renamed page, through
/// the graph's file-name codec (the one the page move uses, master
/// b8f73b9af107). `inner` is the link's text between `[[` and `]]`; the
/// result is the stem's range in `inner` and the new stem.
fn file_link_stem_edit(inner: &str, renames: &Renames<'_>) -> Option<Edit> {
    let body = inner.strip_prefix("file:")?;
    let path_end = body.find("][").unwrap_or(body.len());
    let path = &body[..path_end];
    let slash = path.rfind('/').map_or(0, |at| at + 1);
    let file = &path[slash..];
    let stem_len = file.rfind('.').map_or(file.len(), |dot| dot);
    let stem = &file[..stem_len];
    let decoded = crate::model::decode_page_name(stem, renames.file_name_format);
    let to = renames.page(&decoded)?;
    let start = "file:".len() + slash;
    Some(renames.edit(
        start..start + stem_len,
        &decoded,
        &to,
        crate::model::encode_page_name(&to, renames.file_name_format),
    ))
}

#[cfg(test)]
mod tests {
    //! Rename reads a property value for code once, however many query macros
    //! it holds (qs-leftovers-1109 round 2; PC-1, PC-3, PC-5).
    use super::{HashMap, HashSet, VALUE_PARSES};
    use crate::config::FileNameFormat;
    use std::time::{Duration, Instant};

    #[test]
    fn certificates_keep_the_minimum_partition_and_linear_rewrite_budget() {
        for n in [1, 64, 512] {
            let text = "- [[Old]]\n".repeat(n);
            let map = HashMap::from([("old".into(), "New".into())]);
            let start = Instant::now();
            let p = crate::refs::rename_proposal(
                &text,
                false,
                &map,
                &HashMap::new(),
                FileNameFormat::TripleLowbar,
            )
            .unwrap();
            assert_eq!(p.text, "- [[New]]\n".repeat(n));
            assert_eq!(p.trace.inputs.len(), 1);
            assert_eq!(
                p.trace.pieces.len(),
                2 * n + 1,
                "one edit and one intervening protected piece per site, plus the leading piece"
            );
            assert_eq!(p.trace.output.len(), p.trace.pieces.len());
            assert_eq!(p.trace.substitution.len(), n);
            assert!(start.elapsed()<Duration::from_secs(1),"PC-3: 512 references, including partition/render/check, keep the existing one-second rewrite budget");
        }
    }

    #[test]
    fn own_title_rebinding_certifies_only_the_name_span() {
        for (text, want, org) in [
            (
                "TITLE::  Old  \r\n- body\r\n",
                "TITLE::  New  \r\n- body\r\n",
                false,
            ),
            (
                "#+TITLE:  Old  \r\n* body\r\n",
                "#+TITLE:  New  \r\n* body\r\n",
                true,
            ),
        ] {
            let format = if org {
                crate::model::Format::Org
            } else {
                crate::model::Format::Md
            };
            assert_eq!(
                crate::model::rebind_page_title(text, format, "Old", "New").as_deref(),
                Some(want)
            );
            let mut p = crate::refs::rename_proposal(
                text,
                org,
                &HashMap::new(),
                &HashMap::new(),
                FileNameFormat::TripleLowbar,
            )
            .unwrap();
            p.rebind_title(org, "New").unwrap();
            assert_eq!(p.text, want);
            assert_eq!(p.trace.substitution.len(), 1);
            let id = *p.trace.substitution.keys().next().unwrap();
            assert_eq!(p.trace.original(id), "Old");
        }
    }

    #[test]
    fn proposal_reports_each_unrealized_covered_site() {
        use super::Unrealized::*;
        for (text, org, old, to, reason) in [
            ("* heading :Old:\n", true, "Old", "two words", HeadlineTag),
            ("- Old:: value\n", false, "Old", "Two Words", PropertyKey),
            (
                "- {{query (page [[Old]])}}\n",
                false,
                "Old",
                "bad}name",
                StoredQuery,
            ),
            (
                "- {{renderer [[Old]]}}\n",
                false,
                "Old",
                "bad}name",
                MacroLink,
            ),
            (
                "- {{query (page \"Old\n  name\")}}\n",
                false,
                "Old\nname",
                "New",
                CrossPiece,
            ),
        ] {
            let map = HashMap::from([(crate::refs::page_key(old), to.to_owned())]);
            let p = crate::refs::rename_proposal(
                text,
                org,
                &map,
                &HashMap::new(),
                FileNameFormat::TripleLowbar,
            )
            .unwrap();
            assert!(
                p.unrealized.iter().any(|s| s.reason == reason),
                "{reason:?}: {:?}",
                p.unrealized
            );
            p.trace.check(&p.text, &p.trace.substitution).unwrap();
        }
    }

    #[test]
    fn recording_applied_names_reuses_the_rewrite_walk_within_the_budget() {
        let map = vec![("Old".into(), "New".into())];
        for n in [1, 64, 512] {
            let text = "- [[Old/link]] #Old/tag {{renderer [[Old/macro]]}} {{query (page [[Old/query]])}}\n".repeat(n);
            super::BLOCK_WALKS.with(|walks| walks.set(0));
            let plain = crate::refs::rename_rewrite_with_queries(
                &text,
                false,
                &map,
                &[],
                FileNameFormat::TripleLowbar,
                None,
            );
            let walks = super::BLOCK_WALKS.with(|walks| walks.replace(0));
            let mut names = HashSet::new();
            let start = Instant::now();
            let recorded = crate::refs::rename_rewrite_with_queries(
                &text,
                false,
                &map,
                &[],
                FileNameFormat::TripleLowbar,
                Some(&mut names),
            );
            assert!(
                start.elapsed() < Duration::from_secs(1),
                "PC-3: recording 512 blocks must keep the existing one-second rewrite budget"
            );
            assert_eq!(recorded, plain);
            assert_eq!(walks, n);
            assert_eq!(super::BLOCK_WALKS.with(|walks| walks.replace(0)), n);
            assert_eq!(
                names.len(),
                4,
                "repeated edits retain one pair per distinct rewrite"
            );
            for (from, to) in names {
                assert_eq!(to, from.replacen("Old/", "New/", 1));
            }
            let start = Instant::now();
            let proposal = crate::refs::rename_proposal(
                &text,
                false,
                &HashMap::from([("old".into(), "New family".into())]),
                &HashMap::new(),
                FileNameFormat::TripleLowbar,
            )
            .unwrap();
            assert_eq!(super::BLOCK_WALKS.with(|walks| walks.replace(0)), n);
            assert_eq!(
                proposal.trace.wrappers.len(),
                n,
                "one explicit tag wrapper per block"
            );
            proposal.trace.check_shadow(&proposal.text).unwrap();
            assert!(
                start.elapsed() < Duration::from_secs(1),
                "PC-3: ordered certificates keep the one-second rewrite budget"
            );
        }
        // Macro safety rejects these proposed edits; collecting at lookup
        // time would incorrectly report both names as moved.
        let mut names = HashSet::new();
        let text = "- {{renderer [[Old/macro]]}} {{query (page [[Old/query]])}}\n";
        assert_eq!(
            crate::refs::rename_rewrite_with_queries(
                text,
                false,
                &[("Old".into(), "Bad}name".into())],
                &[],
                FileNameFormat::TripleLowbar,
                Some(&mut names)
            ),
            text
        );
        assert!(names.is_empty());
    }

    const OLD: &str = "{{tine-query @page and name = 'Old'}}";
    const NEW: &str = "{{tine-query @page and name = 'New'}}";

    fn rename(text: &str, is_org: bool) -> String {
        crate::refs::rename_rewrite(
            text,
            is_org,
            &[("Old".to_owned(), "New".to_owned())],
            FileNameFormat::TripleLowbar,
        )
    }

    /// Property values read for code since the last call, on this thread. The
    /// counter is thread-local, so tests running in parallel never share it.
    fn value_reads() -> usize {
        VALUE_PARSES.with(|reads| reads.replace(0))
    }

    /// `n` macros side by side in one value, every second one inside a code
    /// span written with `mark`: the value before and after the rename.
    fn value_of(n: usize, mark: &str) -> (String, String) {
        let spelled = |renamed: bool| {
            (0..n)
                .map(|i| match (i % 2, renamed) {
                    (1, _) => format!("{mark}{OLD}{mark}"),
                    (_, true) => NEW.to_owned(),
                    _ => OLD.to_owned(),
                })
                .collect::<Vec<_>>()
                .join(" ")
        };
        (spelled(false), spelled(true))
    }

    /// A value is read once, whether it holds one macro or four hundred, in a
    /// Markdown property and in an Org drawer; the answer is the same as one
    /// read per macro gave: the macros outside code follow, those in code stay.
    #[test]
    fn a_value_with_many_macros_is_read_once() {
        for n in [1, 2, 10, 400] {
            let (md, md_to) = value_of(n, "`");
            let (org, org_to) = value_of(n, "~");
            for (text, want, is_org) in [
                (
                    format!("- x\n  q:: {md}\n"),
                    format!("- x\n  q:: {md_to}\n"),
                    false,
                ),
                (
                    format!("* x\n:PROPERTIES:\n:Q: {org}\n:END:\n"),
                    format!("* x\n:PROPERTIES:\n:Q: {org_to}\n:END:\n"),
                    true,
                ),
            ] {
                value_reads();
                assert_eq!(rename(&text, is_org), want, "n = {n}, org = {is_org}");
                assert_eq!(value_reads(), 1, "n = {n}, org = {is_org}");
            }
        }
    }

    /// A value without a query macro is not read, and neither is a macro in the
    /// block's text.
    #[test]
    fn values_without_a_macro_are_not_read() {
        let text = format!("- {OLD}\n  a:: plain\n  b:: `code`\n  c:: {{{{embed [[Keep]]}}}}\n");
        value_reads();
        assert_eq!(rename(&text, false), text.replace(OLD, NEW));
        assert_eq!(value_reads(), 0);
    }

    /// Exactly one read per value that holds a macro, in a page property and in
    /// block properties, and none for the values and text beside them.
    #[test]
    fn each_value_holding_a_macro_is_read_once() {
        let text = format!(
            "type:: {OLD} {OLD}\n\n- x {OLD}\n  a:: {OLD} `{OLD}` {OLD}\n  b:: plain `code`\n  c:: `{OLD}`\n- y\n  d:: {OLD}\n"
        );
        let want = format!(
            "type:: {NEW} {NEW}\n\n- x {NEW}\n  a:: {NEW} `{OLD}` {NEW}\n  b:: plain `code`\n  c:: `{OLD}`\n- y\n  d:: {NEW}\n"
        );
        value_reads();
        assert_eq!(rename(&text, false), want);
        // type, a, c, d.
        assert_eq!(value_reads(), 4);
    }

    /// PC-1/PC-3 timing budget, in the form of sync_diff_tests.rs's
    /// `similarity_pairing_is_bounded_on_giant_single_line_blocks` (an
    /// `Instant`, a bound, a message naming the rule): a 76 kB value of 2000
    /// macros renames in time linear in its length, about 0.06 s unoptimized
    /// (the bound leaves 16x for a busy host). Reading it once per macro
    /// (round 1) took 11.6 s here.
    #[test]
    fn a_value_with_many_macros_renames_within_the_budget() {
        let (before, after) = value_of(2000, "`");
        let text = format!("- x\n  q:: {before}\n");
        let start = Instant::now();
        let got = rename(&text, false);
        let took = start.elapsed();
        assert_eq!(got, format!("- x\n  q:: {after}\n"));
        assert!(
            took < Duration::from_secs(1),
            "PC-3: a property value is read once for all its macros, not once per macro ({took:?})"
        );
    }
}
