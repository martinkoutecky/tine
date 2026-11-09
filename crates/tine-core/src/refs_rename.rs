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
//! parse per query macro; O(page bytes). Pure.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;

use crate::config::FileNameFormat;
use crate::doc::{property_key_norm, DocBlock};
use crate::outline::{self, OutlineFormat};
use crate::query::macro_text::{
    macro_safe, query_macro_extents, recognizable_macro, split_trailing_map, FormFamily,
};
use crate::reference_evidence::RewriteTarget;
use crate::refs::{is_linkable_property_separator, page_key};

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
}

impl<'a> Renames<'a> {
    /// `pages` maps `page_key(old)` to the new display name; `queries` (keyed
    /// the same way) overrides it for query page-name arguments only.
    pub(crate) fn new(
        pages: &'a HashMap<String, String>,
        queries: Option<&'a HashMap<String, String>>,
        file_name_format: FileNameFormat,
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
}

type Edit = (Range<usize>, String);

/// Rewrite a whole page `text` (`.md` or `.org`). Returns `text` unchanged when
/// nothing names a renamed page.
pub(crate) fn rewrite_page(text: &str, is_org: bool, renames: &Renames<'_>) -> String {
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
            .map(|(range, to)| (original(range.start)..original(range.end), to)),
    )
}

/// Rewrite one block's raw text (a page DTO's block body, as the merge
/// rewrites it).
pub(crate) fn rewrite_block(raw: &str, is_org: bool, renames: &Renames<'_>) -> String {
    let mut block = DocBlock::new(raw);
    block.set_org(is_org);
    splice(raw, settle(block_edits(raw, is_org, &block, renames)))
}

/// Rewrite a page's pre-block text (its page properties; a page DTO's
/// `pre_block`, as the merge rewrites it).
pub(crate) fn rewrite_pre_block(pre: &str, is_org: bool, renames: &Renames<'_>) -> String {
    splice(pre, settle(pre_block_edits(pre, is_org, renames)))
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
fn splice(text: &str, edits: impl DoubleEndedIterator<Item = Edit>) -> String {
    let mut out = text.to_owned();
    for (range, to) in edits.rev() {
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
        for (range, to) in block_edits(&raw, is_org, &block, renames) {
            let at = pieces.partition_point(|(start, _)| *start <= range.start) - 1;
            let (piece_start, page_start) = pieces[at];
            let piece_end = pieces.get(at + 1).map_or(raw.len(), |(start, _)| start - 1);
            if range.end <= piece_end {
                let shift = page_start - piece_start;
                edits.push((range.start + shift..range.end + shift, to));
            }
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
        edits.extend(
            block_edits(raw, is_org, &block, renames)
                .into_iter()
                .map(|(r, to)| (r.start + range.start..r.end + range.start, to)),
        );
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
    block_edits(&raw, is_org, &block, renames)
        .into_iter()
        .filter_map(|(range, to)| {
            let (start, _, at) = pieces
                .iter()
                .find(|(start, end, _)| *start <= range.start && range.end <= *end)?;
            Some((range.start - start + at..range.end - start + at, to))
        })
        .collect()
}

/// The edits of one block, in its raw offsets (unsettled).
fn block_edits(raw: &str, is_org: bool, block: &DocBlock, renames: &Renames<'_>) -> Vec<Edit> {
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
                        edits.push((range, key));
                    }
                }
            }
            "implicit_linkable_property" => {
                if let Some(to) = renames.page(&reference.name) {
                    edits.push((range, member_spelling(&to)));
                }
            }
            "explicit_tag" => {
                let Some(to) = renames.page(&reference.name) else {
                    continue;
                };
                if node.starts_with('#') {
                    edits.push((range, tag_for(&to)));
                } else if headline_tag_ok(&to) {
                    // An Org headline tag `:name:`: the range is the name.
                    edits.push((range, to.into_owned()));
                }
            }
            "explicit_link" => link_edits(raw, range, &reference.name, renames, &mut edits),
            "explicit_nested_link" => nested_edits(node, range.start, renames, &mut edits),
            "explicit_embed" => {
                if let (Some(open), Some(close)) = (node.find("[["), node.rfind("]]")) {
                    if open + 2 <= close {
                        if let Some(to) = renames.page(&node[open + 2..close]) {
                            edits.push((range.start + open + 2..range.start + close, to.into()));
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
                    if let Some((stem, to)) = raw
                        .get(inner.clone())
                        .and_then(|inner| file_link_stem_edit(inner, renames))
                    {
                        edits.push((inner.start + stem.start..inner.start + stem.end, to));
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
                    edits.extend(local.into_iter().filter(|(_, to)| !to.contains('}')));
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
            edits.push((at + inner.start..at + inner.end, to.into_owned()));
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
            edits.push((
                range.start + inner.start..range.start + inner.end,
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

/// Whether `at` is inside code (a backtick span, an Org `=…=` or `~…~` span)
/// written in a property's value. The outline parser reads a property line as
/// one `Properties` node and keeps each value as text, which is why its
/// literal regions end at the node; the app shows the value as inline text
/// (`PagePropertyValue`), so the same value parsed on its own, as the parser
/// would read it as text, says what is code there. One small parse, only for a
/// macro that sits inside a property value.
fn in_property_value_code(
    raw: &str,
    regions: &crate::block_regions::BlockRegions,
    at: usize,
    is_org: bool,
) -> bool {
    regions
        .properties
        .iter()
        .find(|property| property.value_range.contains(at))
        .is_some_and(|property| {
            crate::block_regions::parse(property.value_range.slice(raw), is_org)
                .literal_at(at - property.value_range.0)
        })
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
    for extent in query_macro_extents(raw) {
        if regions.literal_at(extent.start)
            || in_property_value_code(raw, regions, extent.start, is_org)
        {
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
            local.push((form_start + range.start..form_start + range.end, spelled));
        }
        if local.is_empty() {
            continue;
        }
        let mut argument = extent.argument.clone();
        for (range, to) in local.iter().rev() {
            argument.replace_range(range.start - argument_start..range.end - argument_start, to);
        }
        if macro_safe(&argument, family).is_ok()
            && recognizable_macro(&extent.name, &argument).is_ok()
        {
            edits.extend(local);
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
fn member_spelling(to: &str) -> String {
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
    Some((
        start..start + stem_len,
        crate::model::encode_page_name(&to, renames.file_name_format),
    ))
}
