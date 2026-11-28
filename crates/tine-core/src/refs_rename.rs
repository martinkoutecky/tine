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
//! O7 also leaves and reports a reference inside another reference's name:
//! changing it would repoint that enclosing reference. This follows the
//! confirmed model's nested-reference ruling; Logseq's additional rename
//! of the enclosing pages is the pending decision [CM-NESTED].
//!
//! **Offsets.** Each block's parser offsets are in its raw text (the header line
//! after its structural prefix, continuation lines dedented); they are mapped
//! back to the page text line by line, and an LF offset back to the file's own
//! bytes past each CRLF, so every byte that is not a rewritten name is kept.
//!
//! Cost: one source parse per distinct raw block, shared with the observation;
//! local candidate confirmations and query/property reads remain bounded by
//! their sites. O(page bytes). Pure.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::ops::Range;

use crate::config::FileNameFormat;
use crate::doc::{property_key_norm, DocBlock};
use crate::outline::{self, OutlineFormat};
use crate::query::macro_text::{macro_safe, query_macro_extents, recognizable_macro, FormFamily};
use crate::reference_evidence::RewriteTarget;
use crate::refs::{is_linkable_property_separator, page_key};
use crate::text_trace::{TextTrace, TraceError};

/// A covered site model r6 O7 reports: one the span writer cannot faithfully
/// realize (left as it is), or a property key written in key spelling that
/// differs from the new name. B3 decides admission; this writer reports the
/// evidence without refusing the site.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Site {
    pub range: Range<usize>,
    pub reason: Unrealized,
    /// What kind of name site it is (model `SiteKind`).
    pub kind: SiteKind,
}

/// Why a covered substitution did not have a faithful native spelling, or
/// (`KeySpelling`) had one only in key spelling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unrealized {
    HeadlineTag,
    /// A property key cannot hold the new name (left as it is).
    PropertyKey,
    StoredQuery,
    CrossPiece,
    MacroLink,
    /// O7: "a property key is written in key spelling (lower-cased,
    /// whitespace hyphenated), reported when that differs from the new name".
    /// The key is rewritten.
    KeySpelling,
    /// No faithful label spelling or contiguous name run; left and reported (O7).
    LinkLabel,
    /// O7: this reference's native syntax cannot spell the new name.
    NativeSpelling,
}

/// The kind of a reported name site (the model's `SiteKind`, the kinds a
/// report can carry: task markers and priorities are fixed sites, O2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SiteKind {
    Link,
    Tag,
    Embed,
    PropertyKey,
    PropertyValue,
    Query,
    MacroArg,
}

impl SiteKind {
    /// The kind of an explicit reference by its rule.
    fn of_rule(rule: &str) -> Self {
        match rule {
            "explicit_tag" => SiteKind::Tag,
            "explicit_embed" => SiteKind::Embed,
            "explicit_property_key" => SiteKind::PropertyKey,
            "implicit_linkable_property" => SiteKind::PropertyValue,
            _ => SiteKind::Link,
        }
    }
}

/// The kind of the name site at `range` of `block` (raw offsets): the
/// explicit reference holding it, else the query macro holding it, else a
/// macro argument (the rewrite targets no reference selects).
fn site_kind_at(raw: &str, block: &DocBlock, range: &Range<usize>) -> SiteKind {
    let within = |outer: &Range<usize>| outer.start <= range.start && range.end <= outer.end;
    if let Some(reference) = block
        .projection()
        .reference_source()
        .explicit
        .iter()
        .find(|reference| within(&reference.range))
    {
        return SiteKind::of_rule(reference.rule);
    }
    if query_macro_extents(raw)
        .iter()
        .any(|extent| within(&(extent.start..extent.end)))
    {
        SiteKind::Query
    } else {
        SiteKind::MacroArg
    }
}

/// One physical file's span rewrite and complete-text evidence, all ephemeral.
#[derive(Clone, Debug)]
pub struct RenameProposal {
    pub text: String,
    pub trace: TextTrace,
    pub unrealized: Vec<Site>,
    pub rewritten: HashSet<(String, String)>,
    /// The model's reading of the proposal's input text, from the page parse
    /// the rewrite read; `None` on a proposal rebuilt from a prepared step.
    pub observed: Option<std::sync::Arc<crate::cm_check::Content>>,
}

impl RenameProposal {
    /// Rebind the own title through the reader's title-name span. The title
    /// is one edit, and this is its one producer: `name` (the page's new
    /// identity) with every reference written inside the title rebound, as
    /// it is everywhere else (`old/[[old]]` for `Old/[[Old]]` renamed to
    /// `old`). `name` keeps a tail of the old title verbatim (its namespace
    /// suffix); a reference there is rebound in place. A rebound reference
    /// outside that tail fails encoding/partition validation. Rebinding that changes the title's identity
    /// (`Old/[[Old]]` renamed to `New`: the name says `New/[[Old]]`, its
    /// reference says `[[New]]`), has no faithful image: `TitleIdentity` (Q-TITLE).
    /// Key, spacing and terminators remain protected, as for reference edits.
    /// O(page pieces).
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
        self.rebind_title_at(range, name)
    }

    /// Rebind using the native title coordinates already returned by the
    /// reader. Same span writer as `rebind_title`, without reading the input.
    pub(crate) fn rebind_title_at(
        &mut self,
        range: Range<usize>,
        name: &str,
    ) -> Result<(), TraceError> {
        let original = &self.trace.inputs[0];
        let kept = common_suffix_len(&original[range.clone()], name);
        let tail = range.end - kept;
        let mut edits = Vec::new();
        let mut wrappers = HashSet::new();
        let mut rebound = String::new();
        let mut at = tail;
        for (id, p) in self.trace.pieces.iter().enumerate() {
            let Some(to) = self.trace.substitution.get(&id) else {
                continue;
            };
            let wrapped = self.trace.wrappers.contains(&id);
            if p.range.end <= range.start || p.range.start >= range.end {
                if wrapped {
                    wrappers.insert(p.range.clone());
                }
                edits.push((p.range.clone(), to.clone()));
            } else if p.range.start < range.start || p.range.end > range.end {
                return Err(TraceError::Partition);
            } else if p.range.start < tail {
                return Err(TraceError::Substitution);
            } else {
                rebound.push_str(&original[at..p.range.start]);
                let (open, close) = if wrapped { ("[[", "]]") } else { ("", "") };
                rebound.push_str(open);
                rebound.push_str(to);
                rebound.push_str(close);
                at = p.range.end;
            }
        }
        rebound.push_str(&original[at..range.end]);
        let title = format!("{}{rebound}", &name[..name.len() - kept]);
        if !crate::refs::same_page(&title, name) {
            return Err(TraceError::TitleIdentity);
        }
        let at = edits.partition_point(|(r, _)| r.start < range.start);
        edits.insert(at, (range, title));
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

/// Byte length of the longest common suffix of `a` and `b`, at a character
/// boundary of both.
fn common_suffix_len(a: &str, b: &str) -> usize {
    a.chars()
        .rev()
        .zip(b.chars().rev())
        .take_while(|(x, y)| x == y)
        .map(|(x, _)| x.len_utf8())
        .sum()
}

/// One rename, prepared once per text: old page key to new display name, and
/// the property-key fold of each old name (a property key references the page
/// whose name folds to it, `reference_evidence::visit_reference_targets`).
pub(crate) struct Renames<'a> {
    pages: &'a HashMap<String, String>,
    keys: HashMap<String, &'a str>,
    file_name_format: FileNameFormat,
    /// The graph's journal format: a site spelled as a day stays (O2).
    days: Option<&'a crate::date::JournalFormat>,
    record: bool,
    unrealized: std::cell::RefCell<Vec<Site>>,
}

/// Select a target-only page even when O7 leaves every target unchanged.
pub(crate) fn target_needs_report_or_write(
    raw: &str,
    org: bool,
    pre: bool,
    renames: &Renames<'_>,
) -> bool {
    let text = if pre {
        rewrite_pre_block(raw, org, renames, None)
    } else {
        rewrite_block(raw, org, renames, None)
    };
    text != raw || !renames.unrealized.borrow().is_empty()
}

impl<'a> Renames<'a> {
    /// `pages` maps `page_key(old)` to the new display name.
    pub(crate) fn new(
        pages: &'a HashMap<String, String>,
        format: crate::refs::RenameFormat<'a>,
        record: bool,
    ) -> Self {
        let keys = pages
            .iter()
            .map(|(old, to)| (property_key_norm(old), to.as_str()))
            .collect();
        Self {
            pages,
            keys,
            file_name_format: format.names,
            days: format.days,
            record,
            unrealized: Default::default(),
        }
    }

    fn page(&self, name: &str) -> Option<Cow<'a, str>> {
        crate::refs::renamed_site_name(name, self.pages, self.days)
    }

    /// The new name of a property key: the page it names is the one whose
    /// name folds to it, so the one O1 answer (`renamed_page_name`, prefix
    /// substitution included: a key under the source follows like a link)
    /// runs over the folded map, through the day pin (model r6 O1, O7).
    fn key(&self, name: &str) -> Option<Cow<'_, str>> {
        crate::refs::pinned_site(name, self.days, || {
            crate::refs::renamed_page_name(&property_key_norm(&page_key(name)), &self.keys)
        })
    }

    fn edit(&self, range: Range<usize>, name: &str, to: &str, spelling: String) -> Edit {
        (
            range,
            spelling,
            self.record
                .then(|| Box::new((name.to_owned(), to.to_owned()))),
        )
    }

    fn miss(&self, range: Range<usize>, reason: Unrealized, kind: SiteKind) {
        self.unrealized.borrow_mut().push(Site {
            range,
            reason,
            kind,
        });
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
    let readings = crate::reference_evidence::RenameReadings::default();
    let lf = crate::doc::normalize_line_endings(text);
    let edits = page_edits(&lf, is_org, renames, &readings, None);
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

/// `rewrite_page` with a byte partition, omitted sites and an input observation.
/// Both consumers share the file-local evidence from each block's first parse.
pub(crate) fn proposal(
    text: &str,
    is_org: bool,
    renames: &Renames<'_>,
) -> Result<RenameProposal, TraceError> {
    proposal_read(text, is_org, renames, false, |_, _| ()).map(|(proposal, ())| proposal)
}

pub(crate) fn described_proposal(
    text: &str,
    is_org: bool,
    renames: &Renames<'_>,
) -> Result<(RenameProposal, crate::doc::native::NativeDocument), TraceError> {
    let (mut proposal, mut native) = proposal_read(text, is_org, renames, true, |doc, map| {
        crate::doc::native::NativeDocument::from_read(text, is_org, doc, map.expect("mapped read"))
    })?;
    if let Some(mapped) = native.title.as_ref().and_then(|title| {
        crate::refs::renamed_page_name(title, renames.pages).map(|n| n.into_owned())
    }) {
        if let Some(range) = &native.title_range {
            proposal.rebind_title_at(range.clone(), &mapped)?;
        }
    }
    if is_org {
        standing_header_references(&mut native, text, &proposal.trace)?;
    }
    Ok((proposal, native))
}

/// The references an Org header entry carries where a merge places it alone
/// (O6's leftover): an entry this rename edits, and a non-field line, are
/// read as they will stand, the edited text in its own drawer, through the
/// pre-block reader. A non-field line can sit inside a construct its
/// neighbours open (a quote), which its leftover line leaves; a whole-title
/// replacement can remove or introduce inner references. An unedited field
/// keeps the document reader's reading in context. Linear in header lines
/// and read entry bytes; transient entry buffers.
fn standing_header_references(
    native: &mut crate::doc::native::NativeDocument,
    text: &str,
    trace: &crate::text_trace::TextTrace,
) -> Result<(), TraceError> {
    let edits: Vec<_> = trace
        .pieces
        .iter()
        .enumerate()
        .take_while(|(_, p)| p.range.start < native.end)
        .filter_map(|(id, p)| trace.substitution.get(&id).map(|s| (&p.range, s)))
        .collect();
    let mut cursor = 0;
    for n in 0..native.lines.len() {
        let (line, edit) = (&native.lines[n], |c: usize| edits.get(c).map(|(r, _)| *r));
        while edit(cursor).is_some_and(|r| r.end <= line.content.start) {
            cursor += 1;
        }
        let field = line.property.is_some() || line.drawer.is_some() || line.frame || line.blank;
        if field && !edit(cursor).is_some_and(|r| r.start < line.content.end) {
            continue;
        }
        let mut raw = String::new();
        let drawer = line.drawer.map(|d| native.drawers[d]);
        if let Some((open, _)) = drawer {
            raw.push_str(&text[native.lines[open].content.clone()]);
        }
        let mut at = line.content.start;
        while let Some((range, value)) = edits
            .get(cursor)
            .filter(|(r, _)| r.start < line.content.end)
        {
            if range.start < at || range.end > line.content.end {
                return Err(TraceError::Partition);
            }
            raw.push_str(&text[at..range.start]);
            raw.push_str(value);
            at = range.end;
            cursor += 1;
        }
        let close = drawer.map(|(_, close)| native.lines[close].content.clone());
        for part in [Some(at..line.content.end), close].into_iter().flatten() {
            raw.push_str(&text[part]);
            if crate::doc::line_terminator(&raw).is_empty() {
                raw.push('\n');
            }
        }
        let raw = crate::org::lone_cr_to_lf(&raw);
        let entries = crate::page_properties::pre_block_projections(&raw, true);
        let mut references = Vec::new();
        for (_, block) in entries {
            for r in block.projection().reference_source().explicit {
                if !matches!(r.rule, "explicit_task_marker" | "explicit_priority") {
                    let (name, rule) = (r.name.clone(), r.rule);
                    references.push(crate::doc::native::NativeReference { name, rule });
                }
            }
        }
        native.lines[n].references = references;
    }
    Ok(())
}

fn proposal_read<T>(
    text: &str,
    is_org: bool,
    renames: &Renames<'_>,
    mapped: bool,
    carry: impl FnOnce(crate::doc::Document, Option<crate::doc::SourceMap>) -> T,
) -> Result<(RenameProposal, T), TraceError> {
    let readings = crate::reference_evidence::RenameReadings::default();
    let crate::doc::Outlined {
        lf,
        document: doc,
        headers,
        map,
    } = crate::doc::parse_outlined(text, is_org, mapped);
    let format = if is_org {
        crate::model::Format::Org
    } else {
        crate::model::Format::Md
    };
    let observed =
        crate::cm_check::observe_with_readings(&doc, format, renames.file_name_format, &readings);
    let edits = if renames.pages.is_empty() {
        Vec::new()
    } else {
        page_edits(&lf, is_org, renames, &readings, Some((&headers, &doc)))
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
    Ok((
        RenameProposal {
            text,
            trace,
            unrealized,
            rewritten,
            observed: Some(std::sync::Arc::new(observed)),
        },
        carry(doc, map),
    ))
}

/// Rewrite one held block's raw text for rename inspection.
pub(crate) fn rewrite_block(
    raw: &str,
    is_org: bool,
    renames: &Renames<'_>,
    rewritten: Option<&mut HashSet<(String, String)>>,
) -> String {
    let readings = crate::reference_evidence::RenameReadings::default();
    let mut block = DocBlock::new(raw);
    block.set_org(is_org);
    splice(
        raw,
        settle(block_edits(raw, is_org, &block, renames, &readings)),
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
    let readings = crate::reference_evidence::RenameReadings::default();
    splice(
        pre,
        settle(pre_block_edits(pre, is_org, renames, &readings)),
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

/// Settled LF page edits. `parsed` supplies the outline and blocks; `readings`
/// shares their reader evidence with the check's input observation.
fn page_edits(
    text: &str,
    is_org: bool,
    renames: &Renames<'_>,
    readings: &crate::reference_evidence::RenameReadings,
    parsed: Option<(&[outline::Header], &crate::doc::Document)>,
) -> Vec<Edit> {
    #[cfg(feature = "test-counters")]
    crate::text_trace::work::page_walk();
    let format = if is_org {
        OutlineFormat::Org
    } else {
        OutlineFormat::Markdown
    };
    // An outline lsdoc cannot represent has no blocks: the page parse reads the
    // whole text as the pre-block (`doc::parse`), and so does the rename.
    let headers = match parsed {
        Some((headers, _)) => headers.to_vec(),
        None => outline::headers(text, format).unwrap_or_default(),
    };
    let mut parsed_blocks = Vec::new();
    if let Some((_, doc)) = parsed {
        fn flat<'d>(blocks: &'d [DocBlock], out: &mut Vec<&'d DocBlock>) {
            for block in blocks {
                out.push(block);
                flat(&block.children, out);
            }
        }
        flat(&doc.roots, &mut parsed_blocks);
    }
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
        edits.extend(pre_block_edits(&text[..pre_end], is_org, renames, readings));
    }
    for (n, header) in headers.iter().enumerate() {
        let end = headers.get(n + 1).map_or(lines.len(), |next| next.line);
        // Mirror `outline::blocks`: the header line after its prefix, then each
        // continuation dedented; remember where each piece starts in the page.
        let first_line = &lines[header.line][header.prefix_len..];
        let mut raw = first_line.to_string();
        let mut pieces = vec![(0usize, line_starts[header.line] + header.prefix_len)];
        for (k, line) in lines.iter().enumerate().take(end).skip(header.line + 1) {
            let kept = crate::doc::continuation_raw(line, header.content_indent);
            raw.push('\n');
            pieces.push((raw.len(), line_starts[k] + line.len() - kept.len()));
            raw.push_str(kept);
        }
        let own;
        let block = match parsed_blocks.get(n).filter(|block| block.raw() == raw) {
            Some(block) => *block,
            None => {
                let mut block = DocBlock::new(raw.as_str());
                block.set_org(is_org);
                own = block;
                &own
            }
        };
        let misses = renames.unrealized.borrow().len();
        for (range, to, name) in block_edits(&raw, is_org, block, renames, readings) {
            let at = pieces.partition_point(|(start, _)| *start <= range.start) - 1;
            let (piece_start, page_start) = pieces[at];
            let piece_end = pieces.get(at + 1).map_or(raw.len(), |(start, _)| start - 1);
            if range.end <= piece_end {
                let shift = page_start - piece_start;
                edits.push((range.start + shift..range.end + shift, to, name));
            } else {
                let kind = site_kind_at(&raw, block, &range);
                renames.miss(range, Unrealized::CrossPiece, kind);
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
fn pre_block_edits(
    pre: &str,
    is_org: bool,
    renames: &Renames<'_>,
    readings: &crate::reference_evidence::RenameReadings,
) -> Vec<Edit> {
    let mut edits = property_edits(pre, is_org, renames, readings);
    if let Some(range) = crate::page_properties::preamble_content_range(pre, is_org) {
        let raw = &pre[range.clone()];
        let mut block = DocBlock::new(raw);
        block.set_org(is_org);
        let misses = renames.unrealized.borrow().len();
        edits.extend(
            block_edits(raw, is_org, &block, renames, readings)
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
fn property_edits(
    pre: &str,
    is_org: bool,
    renames: &Renames<'_>,
    readings: &crate::reference_evidence::RenameReadings,
) -> Vec<Edit> {
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
    let edits = block_edits(&raw, is_org, &block, renames, readings);
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
                let kind = site_kind_at(&raw, &block, &range);
                renames.miss(0..pre.len(), Unrealized::CrossPiece, kind);
                return None;
            };
            Some((range.start - start + at..range.end - start + at, to, name))
        })
        .collect()
}

/// The edits of one block, in its raw offsets (unsettled).
fn block_edits(
    raw: &str,
    is_org: bool,
    block: &DocBlock,
    renames: &Renames<'_>,
    readings: &crate::reference_evidence::RenameReadings,
) -> Vec<Edit> {
    #[cfg(test)]
    BLOCK_WALKS.with(|walks| walks.set(walks.get() + 1));
    let source = readings.read(block);
    let projection = block.projection();
    let mut edits = Vec::new();
    for (index, reference) in source.explicit.iter().enumerate() {
        // A property member in a macro is rewritten as its macro argument.
        if reference.slot.encoding || reference.slot.in_macro {
            continue;
        }
        let range = reference.range.clone();
        let Some(node) = raw.get(range.clone()) else {
            continue;
        };
        if reference.slot.nested {
            if renames.page(&reference.name).is_some() {
                renames.miss(
                    reference.slot.extent.clone(),
                    Unrealized::NativeSpelling,
                    SiteKind::of_rule(reference.rule),
                );
            }
            continue;
        }
        match reference.rule {
            // Permanent exceptions (D-20261019-M2(4)).
            "explicit_task_marker" | "explicit_priority" => {}
            // OG `replace-property-ref!`: the new name lower-cased, whitespace
            // runs hyphenated (D-20261019-M2(4), manager ruling Q1).
            "explicit_property_key" => {
                // The key names its page by page identity (`Über::` is page
                // "über"); `keys` is keyed by the old name's key spelling.
                if let Some(to) = renames.key(&reference.name) {
                    if let Some(key) = property_key_spelling(&to) {
                        if key != *to {
                            renames.miss(
                                range.clone(),
                                Unrealized::KeySpelling,
                                SiteKind::PropertyKey,
                            );
                        }
                        edits.push(renames.edit(range, &reference.name, &to, key));
                    } else {
                        renames.miss(range, Unrealized::PropertyKey, SiteKind::PropertyKey);
                    }
                }
            }
            "implicit_linkable_property" => {
                if let Some(to) = renames.page(&reference.name) {
                    if let Some(spelling) = member_spelling(&to, is_org) {
                        edits.push(renames.edit(range, &reference.name, &to, spelling));
                    } else {
                        renames.miss(range, Unrealized::NativeSpelling, SiteKind::PropertyValue);
                    }
                }
            }
            "explicit_tag" => {
                let Some(to) = renames.page(&reference.name) else {
                    continue;
                };
                if node.starts_with('#') {
                    let bracketed = node.starts_with("#[[") && node.ends_with("]]");
                    let Some(spelling) = tag_spelling(&to, bracketed, is_org) else {
                        renames.miss(range, Unrealized::NativeSpelling, SiteKind::Tag);
                        continue;
                    };
                    // The reader supplied the old name extent. Strip only
                    // the printer's new wrappers that the input already
                    // carries; a bare tag's new brackets remain an insertion.
                    let spelling = if reference.slot.extent.start == range.start + 3 {
                        spelling[3..spelling.len() - 2].to_owned()
                    } else {
                        spelling[1..].to_owned()
                    };
                    edits.push(renames.edit(
                        reference.slot.extent.clone(),
                        &reference.name,
                        &to,
                        spelling,
                    ));
                } else if headline_tag_ok(&to) {
                    // An Org headline tag `:name:`: the range is the name.
                    edits.push(renames.edit(range, &reference.name, &to, to.to_string()));
                } else {
                    renames.miss(range, Unrealized::HeadlineTag, SiteKind::Tag);
                }
            }
            "explicit_link" | "explicit_nested_link" => link_edits(
                raw,
                reference,
                source.explicit[index + 1..].iter(),
                is_org,
                renames,
                &mut edits,
            ),
            "explicit_embed" => {
                if let Some(slot) = reference.slot.run() {
                    if let Some(to) = renames.page(&reference.name) {
                        edits.push(renames.edit(
                            slot.clone(),
                            &reference.name,
                            &to,
                            to.to_string(),
                        ));
                    }
                } else if renames.page(&reference.name).is_some() {
                    renames.miss(range, Unrealized::NativeSpelling, SiteKind::Embed);
                }
            }
            _ => {}
        }
    }
    edits.extend(query_edits(raw, &projection.regions, is_org, renames));
    for (index, target) in source.targets.iter().cloned().enumerate() {
        match target {
            RewriteTarget::FileLink(range) => {
                edits.extend(file_link_stem_edit(raw, range, renames));
            }
            // A `}` would end the macro early: retain its old bytes.
            RewriteTarget::MacroLink(reference) => {
                let misses = renames.unrealized.borrow().len();
                let mut local = Vec::new();
                let inner = source.targets[index + 1..]
                    .iter()
                    .filter_map(|target| match target {
                        RewriteTarget::MacroLink(r) => Some(r),
                        _ => None,
                    });
                link_edits(raw, &reference, inner, is_org, renames, &mut local);
                for site in &mut renames.unrealized.borrow_mut()[misses..] {
                    site.kind = SiteKind::MacroArg;
                }
                for edit in local {
                    if edit.1.contains('}') {
                        renames.miss(edit.0, Unrealized::MacroLink, SiteKind::MacroArg);
                    } else {
                        edits.push(edit);
                    }
                }
            }
        }
    }
    edits
}

/// A link node `[[x]]`, Org `[[x][label]]`, Markdown `[label]([[x]])`, or a
/// file link named by its label (Org `[[file:…][label]]`, Markdown
/// `[label](file:…)` or `[label](../pages/x.md)`): respell the name.
fn link_edits<'a>(
    raw: &str,
    reference: &crate::reference_evidence::SlottedPageRef,
    following: impl Iterator<Item = &'a crate::reference_evidence::SlottedPageRef>,
    is_org: bool,
    renames: &Renames<'_>,
    edits: &mut Vec<Edit>,
) {
    let range = reference.range.clone();
    let name = &reference.name;
    let Some(to) = renames.page(name) else {
        return;
    };
    if reference.slot.nested {
        renames.miss(
            reference.slot.extent.clone(),
            Unrealized::NativeSpelling,
            SiteKind::Link,
        );
        return;
    }
    let node = &raw[range.clone()];
    if let Some(label) = crate::refs::FileLabel::read(raw, reference, is_org) {
        match label.spelling(&to) {
            Some(spelling) => {
                let run = label.run.expect("a spelling has a name run");
                edits.push(renames.edit(
                    range.start + run.start..range.start + run.end,
                    name,
                    &to,
                    spelling,
                ));
            }
            None => renames.miss(
                range.start + label.label.start..range.start + label.label.end,
                Unrealized::LinkLabel,
                SiteKind::Link,
            ),
        }
        return;
    }
    if let Some(at) = reference.slot.run() {
        let inner = at.start - range.start..at.end - range.start;
        let mut following = following
            .take_while(|r| r.range.start < range.end)
            .filter(|r| !r.slot.encoding && r.range.end <= range.end)
            .peekable();
        let prefix = if following.peek().is_some() {
            crate::refs::renamed_site_prefix(name, renames.pages, renames.days)
        } else {
            0
        };
        let expected_inner: Vec<_> = crate::refs::label::retained_inner(
            at.clone(),
            prefix,
            following.map(|r| (r.range.clone(), r)),
        )
        .map(|(_, r)| (r.name.as_str(), &raw[r.range.clone()]))
        .collect();
        match crate::refs::label::reference_spelling(
            node,
            inner,
            &to,
            is_org,
            reference.rule,
            &expected_inner,
        ) {
            Some(spelling) => edits.push(renames.edit(at.clone(), name, &to, spelling)),
            None => renames.miss(at.clone(), Unrealized::NativeSpelling, SiteKind::Link),
        }
    } else {
        renames.miss(
            reference.slot.extent.clone(),
            Unrealized::NativeSpelling,
            SiteKind::Link,
        );
    }
}

#[cfg(test)]
thread_local! {
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
    for query in crate::query::page_names::macro_page_names(raw, regions, is_org) {
        let (family, argument_start) = (query.family, query.argument_start);
        let mut local: Vec<Edit> = Vec::new();
        for (range, name) in query.names {
            let Some(to) = renames.page(&name) else {
                continue;
            };
            let token = &raw[range.clone()];
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
            local.push(renames.edit(range, &name, &to, spelled));
        }
        if local.is_empty() {
            continue;
        }
        let mut argument = query.argument;
        for (range, to, _) in local.iter().rev() {
            argument.replace_range(range.start - argument_start..range.end - argument_start, to);
        }
        if macro_safe(&argument, family).is_ok()
            && recognizable_macro(&query.name, &argument).is_ok()
        {
            edits.extend(local);
        } else {
            for (range, _, _) in local {
                renames.miss(range, Unrealized::StoredQuery, SiteKind::Query);
            }
        }
    }
    edits
}

/// An OG DSL page-name token respelled for `to`, keeping its form.
pub(crate) fn og_token_spelling(token: &str, to: &str) -> String {
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
pub(crate) fn member_spelling(to: &str, org: bool) -> Option<String> {
    let candidate = if to.contains(is_linkable_property_separator)
        || to.starts_with(['#', '"'])
        || to.contains("[[")
        || to.contains("]]")
    {
        format!("[[{}]]", crate::refs::label::bracket_name(to, org)?)
    } else {
        to.to_owned()
    };
    let reads = |value: &str| {
        // Alias members use the same separated-member reader as linkable
        // properties; this probe is parsed in memory, never written.
        let node = if org {
            format!("h\n:PROPERTIES:\n:alias: {value}\n:END:")
        } else {
            format!("alias:: {value}")
        };
        crate::refs::label::reads_reference(&node, org, "implicit_linkable_property", to, None)
            || crate::refs::label::reads_reference(&node, org, "explicit_link", to, None)
    };
    if reads(&candidate) {
        Some(candidate)
    } else {
        let wrapped = format!("[[{}]]", crate::refs::label::bracket_name(to, org)?);
        reads(&wrapped).then_some(wrapped)
    }
}

/// OG `replace-property-ref!`'s key: lower-cased, whitespace runs hyphenated.
/// `None` when that is no property key (empty, or holding a `:`).
pub(crate) fn property_key_spelling(to: &str) -> Option<String> {
    let key = to
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-");
    (!key.is_empty() && !key.contains(':')).then_some(key)
}

/// Whether the parser reads one Org headline tag; otherwise O7 leaves and reports it.
pub(crate) fn headline_tag_ok(to: &str) -> bool {
    !to.is_empty()
        && !to.contains(['\n', '\r'])
        && crate::refs::label::reads_reference(
            &format!("h :{to}:"),
            true,
            "explicit_tag",
            to,
            Some(3..3 + to.len()),
        )
}

/// Keep a bracketed tag's form; otherwise try the existing bare/wrapped
/// convention. The parser confirms the whole tag and its actual name.
pub(crate) fn tag_spelling(to: &str, bracketed: bool, org: bool) -> Option<String> {
    if !bracketed {
        let candidate = tag_for(to);
        if crate::refs::label::reads_reference(
            &candidate,
            org,
            "explicit_tag",
            to,
            Some(0..candidate.len()),
        ) {
            return Some(candidate);
        }
    }
    crate::refs::label::reference_spelling("#[[Old]]", 3..6, to, org, "explicit_tag", &[])
        .map(|name| format!("#[[{name}]]"))
}

/// Respell an Org `file:` link path whose stem names a renamed page, through
/// the graph's file-name codec (the one the page move uses, master
/// b8f73b9af107). `link` is the link's range in `raw`; the result is the
/// stem's range in `raw` and the new stem.
fn file_link_stem_edit(raw: &str, link: Range<usize>, renames: &Renames<'_>) -> Option<Edit> {
    let stem = crate::model::org_file_link_stem(raw.get(link.clone())?)?;
    let stem = link.start + stem.start..link.start + stem.end;
    let decoded = crate::model::decode_page_name(&raw[stem.clone()], renames.file_name_format);
    let to = renames.page(&decoded)?;
    let Some(spelling) =
        crate::refs::label::file_stem_spelling(&raw[link.clone()], &to, renames.file_name_format)
    else {
        renames.miss(stem, Unrealized::NativeSpelling, SiteKind::Link);
        return None;
    };
    Some(renames.edit(stem, &decoded, &to, spelling))
}

#[cfg(test)]
mod tests {
    //! Rename reads a property value for code once, however many query macros
    //! it holds (qs-leftovers-1109 round 2; PC-1, PC-3, PC-5).
    use super::{HashMap, HashSet};
    use crate::config::FileNameFormat;
    use crate::query::page_names::VALUE_PARSES;
    use std::time::{Duration, Instant};

    /// O2 per site (round 2 finding 1): page `2026 10 26` is no day, but its
    /// property-key spelling `2026-10-26` is, and a query argument spelled as a
    /// day is a day site too. Both keep their spelling, as a `[[2026-10-26]]`
    /// link does; the ordinary sites around them follow the rename.
    #[test]
    fn a_day_spelled_property_key_stays() {
        let days = crate::date::JournalFormat::default();
        let format = crate::refs::RenameFormat::new(FileNameFormat::TripleLowbar, &days);
        let pages = HashMap::from([("2026 10 26".into(), "New".into())]);
        let text = "- [[2026 10 26]]\n  2026-10-26:: value\n";
        let got = crate::refs::rename_proposal(text, false, &pages, format);
        assert_eq!(got.unwrap().text, "- [[New]]\n  2026-10-26:: value\n");
    }

    /// A rename rewrites exactly a property value's members (the one owner).
    /// `**[[a]]**` is no member (OG's property references are top level), so
    /// it names no page and OG's rename (`page.cljs` `rename-update-refs!`,
    /// which rewrites only the blocks in the page's `:block/_refs`) leaves it;
    /// a macro's link is a member and follows as the macro's argument.
    #[test]
    fn a_rename_rewrites_exactly_the_property_members() {
        let pages = HashMap::from([("a".into(), "New".into())]);
        for (org, text, want) in [
            (
                false,
                "alias:: **[[a]]**\n\n- b\n",
                "alias:: **[[a]]**\n\n- b\n",
            ),
            (true, "#+alias: /[[a]]/\n* b\n", "#+alias: /[[a]]/\n* b\n"),
            (
                false,
                "- b\n  foo:: {{renderer [[a]]}}, {{embed [[a]]}}\n  tags:: x, [[a]]\n",
                "- b\n  foo:: {{renderer [[New]]}}, {{embed [[New]]}}\n  tags:: x, [[New]]\n",
            ),
        ] {
            let got = crate::refs::rename_proposal(text, org, &pages, FileNameFormat::TripleLowbar)
                .unwrap();
            assert_eq!(got.text, want, "{text:?}");
            assert!(got.unrealized.is_empty(), "{text:?}: {:?}", got.unrealized);
        }
    }

    #[test]
    fn a_day_spelled_query_argument_stays() {
        let days = crate::date::JournalFormat::default();
        let format = || crate::refs::RenameFormat::new(FileNameFormat::TripleLowbar, &days);
        let pages = HashMap::from([
            ("other".into(), "New".into()),
            ("2026-10-26".into(), "New".into()),
        ]);
        let text = "- {{query [[2026-10-26]]}} [[Other]]\n";
        let got = crate::refs::rename_proposal(text, false, &pages, format());
        assert_eq!(got.unwrap().text, "- {{query [[2026-10-26]]}} [[New]]\n");
    }

    #[test]
    fn certificates_keep_the_minimum_partition_and_linear_rewrite_budget() {
        for n in [1, 64, 512] {
            let text = "- [[Old]]\n".repeat(n);
            let map = HashMap::from([("old".into(), "New".into())]);
            let start = Instant::now();
            let p = crate::refs::rename_proposal(&text, false, &map, FileNameFormat::TripleLowbar)
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
            let span = crate::model::own_title_span(text, format, "Old").unwrap();
            assert_eq!(&text[span], "Old");
            let mut p = crate::refs::rename_proposal(
                text,
                org,
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

    /// Review r6 F1: a case-only rename of namespace `Old` to `old` reaches a
    /// child whose title holds `[[old]]`; that reference's substitution is
    /// byte-identical and must not overlap the whole-title edit.
    #[test]
    fn rebind_title_ignores_a_byte_identical_reference_inside_the_title() {
        for (text, org, want) in [
            (
                "title:: Old/[[old]]\n- body\n",
                false,
                "title:: old/[[old]]\n- body\n",
            ),
            (
                "#+TITLE: Old/[[old]]\n* body\n",
                true,
                "#+TITLE: old/[[old]]\n* body\n",
            ),
        ] {
            let map = HashMap::from([("old".into(), "old".into())]);
            let mut p = crate::refs::rename_proposal(text, org, &map, FileNameFormat::TripleLowbar)
                .unwrap();
            p.rebind_title(org, "old/[[old]]").unwrap();
            assert_eq!(p.text, want);
        }
    }

    /// Review r7 F1 (the r6 family): the title is one edit holding the new
    /// identity with its inner references rebound, so no inner reference edit
    /// can overlap it; an identity the rebound title would not name refuses.
    #[test]
    fn rebind_title_is_one_edit_with_its_references_rebound() {
        for org in [false, true] {
            let (key, bullet) = if org {
                ("#+TITLE: ", "*")
            } else {
                ("title:: ", "-")
            };
            let page = |title: &str| format!("{key}{title}\n{bullet} [[Old]]\n");
            let case = HashMap::from([("old".into(), "old".into())]);
            let new = HashMap::from([("old".into(), "New".into())]);
            for (map, title, name, want) in [
                (&case, "Old/[[Old]]", "old/[[Old]]", Some("old/[[old]]")),
                (&case, "Old/[[old]]", "old/[[old]]", Some("old/[[old]]")),
                (
                    &case,
                    "Old/[[Old]] [[OLD]]",
                    "old/[[Old]] [[OLD]]",
                    Some("old/[[old]] [[old]]"),
                ),
                (&new, "Old/[[X]]", "New/[[X]]", Some("New/[[X]]")),
                (&new, "Old/[[Old]]", "New/[[Old]]", None),
            ] {
                let text = page(title);
                let mut p =
                    crate::refs::rename_proposal(&text, org, map, FileNameFormat::TripleLowbar)
                        .unwrap();
                let to = map["old"].clone();
                match want {
                    Some(want) => {
                        p.rebind_title(org, name).unwrap();
                        assert_eq!(p.text, format!("{key}{want}\n{bullet} [[{to}]]\n"));
                        let title_edits = p
                            .trace
                            .substitution
                            .keys()
                            .filter(|id| p.trace.original(**id).starts_with("Old/"))
                            .count();
                        assert_eq!(title_edits, 1, "{title}");
                    }
                    None => assert_eq!(
                        p.rebind_title(org, name),
                        Err(crate::text_trace::TraceError::TitleIdentity),
                        "{title}"
                    ),
                }
            }
        }
    }

    #[test]
    fn rebind_title_refuses_an_inner_edit_outside_the_kept_tail() {
        let links = HashMap::from([("old".into(), "New".into())]);
        let mut proposal = crate::refs::rename_proposal(
            "title:: Old/[[Old]]\n- body\n",
            false,
            &links,
            crate::config::FileNameFormat::TripleLowbar,
        )
        .unwrap();
        assert_eq!(
            proposal.rebind_title(false, "New/Child"),
            Err(crate::text_trace::TraceError::Substitution)
        );
    }

    /// Model r6 O1 ("every reference") with O7 ("a site that can spell the new
    /// name is rewritten; one that cannot is left and reported"): a Markdown
    /// file link is a link named by its label (`[Old](file:x.md)`,
    /// `[Old](../pages/x.md)`), so its label is respelled like an Org file
    /// link's. A `]` is written escaped, the spelling the parser reads back as
    /// `a]b` (residual F1 ruling: the parser confirms the spelling, no
    /// character list refuses it); a new name no label spelling holds (a line
    /// break) leaves it as it is and reports the site.
    #[test]
    fn a_markdown_file_link_label_follows_the_rename_or_is_reported() {
        for target in ["file:x.md", "../pages/x.md"] {
            let text = format!("- see [Old]({target})\n");
            let map = HashMap::from([(crate::refs::page_key("Old"), "New".to_owned())]);
            let p = crate::refs::rename_proposal(&text, false, &map, FileNameFormat::TripleLowbar)
                .unwrap();
            assert_eq!(p.text, format!("- see [New]({target})\n"), "{target}");
            assert!(p.unrealized.is_empty(), "{target}: {:?}", p.unrealized);
            let proposal = |to: &str| {
                let map = HashMap::from([(crate::refs::page_key("Old"), to.to_owned())]);
                crate::refs::rename_proposal(&text, false, &map, FileNameFormat::TripleLowbar)
                    .unwrap()
            };
            let p = proposal("a]b");
            assert_eq!(p.text, format!("- see [a\\]b]({target})\n"), "{target}");
            assert!(p.unrealized.is_empty(), "{target}: {:?}", p.unrealized);
            let parsed = crate::render::parse_projection(&p.text, false);
            let names: Vec<String> =
                crate::reference_evidence::project(&p.text, false, &parsed.blocks)
                    .explicit
                    .iter()
                    .map(|reference| reference.name.clone())
                    .collect();
            assert_eq!(names, ["a]b"], "{target}: the reader's name");
            let p = proposal("a\nb");
            assert_eq!(p.text, text, "{target}: no label holds a line break");
            let [site] = p.unrealized.as_slice() else {
                panic!("{target}: one reported site: {:?}", p.unrealized);
            };
            assert_eq!(site.kind, super::SiteKind::Link, "{target}");
            assert_eq!(&text[site.range.clone()], "Old", "{target}");
        }
    }

    /// Review round 2, finding 1 (model r6 O1 with O7; manager ruling): the
    /// evidence names a formatted label by its text (`[**Old**](file:x.md)`
    /// names `Old`). One contiguous run is respelled and the formatting stays;
    /// a name split by markup, or a new name the formatted label cannot hold,
    /// leaves the label and reports it. Never a silent drop.
    #[test]
    fn a_formatted_file_link_label_is_respelled_in_its_run_or_reported() {
        let proposal = |text: &str, org: bool, to: &str| {
            let map = HashMap::from([(crate::refs::page_key("Old"), to.to_owned())]);
            crate::refs::rename_proposal(text, org, &map, FileNameFormat::TripleLowbar).unwrap()
        };
        for (text, org, to, after) in [
            (
                "- [**Old**](file:x.md)\n",
                false,
                "New",
                "- [**New**](file:x.md)\n",
            ),
            (
                "- [*Old*](../pages/x.md)\n",
                false,
                "New",
                "- [*New*](../pages/x.md)\n",
            ),
            (
                "* h [[file:x.org][*Old*]]\n",
                true,
                "New",
                "* h [[file:x.org][*New*]]\n",
            ),
            // Residual F1: the spelling is the one the parser reads back as
            // the name, escaped for `a]b` and as it is for `a*b`, which the
            // parser reads inside the emphasis as `a*b` (the reader's name is
            // asserted in `a_label_respelling_reads_as_the_new_name_or_is_reported`).
            (
                "- [**Old**](file:x.md)\n",
                false,
                "a]b",
                "- [**a\\]b**](file:x.md)\n",
            ),
            (
                "- [**Old**](file:x.md)\n",
                false,
                "a*b",
                "- [**a*b**](file:x.md)\n",
            ),
        ] {
            let p = proposal(text, org, to);
            assert_eq!(p.text, after, "{text}");
            assert!(p.unrealized.is_empty(), "{text}: {:?}", p.unrealized);
        }
        // Split by markup as the parser reads it (it names `Old` through the
        // formatting and the code span).
        for (text, org, to, label) in [
            ("- [**Ol**_d_](file:x.md)\n", false, "New", "**Ol**_d_"),
            ("- [Ol`d`](file:x.md)\n", false, "New", "Ol`d`"),
            ("* h [[file:x.org][*Ol*d]]\n", true, "New", "*Ol*d"),
            // A new name no spelling of the label holds: a line break.
            ("- [**Old**](file:x.md)\n", false, "a\nb", "**Old**"),
        ] {
            let p = proposal(text, org, to);
            assert_eq!(p.text, text, "{text} -> {to}: left as it is");
            let [site] = p.unrealized.as_slice() else {
                panic!("{text} -> {to}: one reported site: {:?}", p.unrealized);
            };
            assert_eq!(site.reason, super::Unrealized::LinkLabel, "{text}");
            assert_eq!(site.kind, super::SiteKind::Link, "{text}");
            assert_eq!(&text[site.range.clone()], label, "{text}");
        }
        // `**Ol**d` is no emphasis to the Markdown parser (a letter follows the
        // closing `**`): the link names the page `**Ol**d`, so it is no site
        // of `Old`, neither respelled nor reported.
        let text = "- [**Ol**d](file:x.md)\n";
        let p = proposal(text, false, "New");
        assert_eq!(p.text, text);
        assert!(p.unrealized.is_empty(), "{:?}", p.unrealized);
    }

    /// Review round 2, residual F1 (manager ruling, 2026-11-16): a label's
    /// new spelling is confirmed by the parser, never by a character list. A
    /// destination holding a Markdown escape (`New\\Tail`, `a\#b`, `a\*b`)
    /// or other inline syntax is either written so that the link reads as
    /// exactly that name, or the label is left and reported: never a label
    /// that reads as another name. Plain and formatted labels, an image, Org.
    #[test]
    fn a_label_respelling_reads_as_the_new_name_or_is_reported() {
        let name_of = |text: &str, org: bool| -> Vec<String> {
            let parsed = crate::render::parse_projection(text, org);
            crate::reference_evidence::project(text, org, &parsed.blocks)
                .explicit
                .iter()
                .filter(|reference| reference.rule == "explicit_link")
                .map(|reference| reference.name.clone())
                .collect()
        };
        let mut respelled = Vec::new();
        for (text, org) in [
            ("- [Old](file:x.md)\n", false),
            ("- [**Old**](file:x.md)\n", false),
            ("- ![Old](../pages/x.md)\n", false),
            ("* h [[file:x.org][Old]]\n", true),
            ("* h [[file:x.org][*Old*]]\n", true),
        ] {
            for to in [
                r"New\\Tail",
                r"a\#b",
                r"a\*b",
                "a*b",
                "a]b",
                "a_b",
                "&amp;",
                "New",
            ] {
                let map = HashMap::from([(crate::refs::page_key("Old"), to.to_owned())]);
                let p = crate::refs::rename_proposal(text, org, &map, FileNameFormat::TripleLowbar)
                    .unwrap();
                if p.text == text {
                    let [site] = p.unrealized.as_slice() else {
                        panic!("{text} -> {to}: left, so one report: {:?}", p.unrealized);
                    };
                    assert_eq!(site.reason, super::Unrealized::LinkLabel, "{text} -> {to}");
                } else {
                    assert_eq!(name_of(&p.text, org), [to], "{text} -> {to}: {}", p.text);
                    assert!(
                        p.unrealized.is_empty(),
                        "{text} -> {to}: {:?}",
                        p.unrealized
                    );
                    respelled.push((text, to));
                }
            }
        }
        // The known failing family is now written faithfully in Markdown:
        // the escaped spelling reads back as the name.
        for to in [r"New\\Tail", r"a\#b", r"a\*b"] {
            assert!(
                respelled.contains(&("- [Old](file:x.md)\n", to)),
                "{to}: {respelled:?}"
            );
        }
        assert!(respelled.contains(&("* h [[file:x.org][Old]]\n", "New")));
    }

    /// Review round 2 follow-up: an image link is a site only when the parser
    /// names a page by it. Then its label follows like a file link's;
    /// otherwise it is no site: untouched and unreported. Either way nothing
    /// the parser named is dropped silently.
    #[test]
    fn an_image_link_is_a_site_only_when_the_parser_names_it() {
        for target in ["../assets/x.png", "file:x.md", "../pages/x.md"] {
            let text = format!("- ![Old]({target})\n");
            let parsed = crate::render::parse_projection(&text, false);
            let named = crate::reference_evidence::project(&text, false, &parsed.blocks)
                .explicit
                .iter()
                .any(|reference| reference.name == "Old");
            let map = HashMap::from([(crate::refs::page_key("Old"), "New".to_owned())]);
            let p = crate::refs::rename_proposal(&text, false, &map, FileNameFormat::TripleLowbar)
                .unwrap();
            let want = if named {
                format!("- ![New]({target})\n")
            } else {
                text.clone()
            };
            assert_eq!(p.text, want, "{target}: parser names it: {named}");
            assert!(p.unrealized.is_empty(), "{target}: {:?}", p.unrealized);
        }
    }

    #[test]
    fn proposal_reports_each_unrealized_covered_site() {
        use super::SiteKind as K;
        use super::Unrealized::*;
        // Model r6 O7: a key respelled into key spelling is reported as such
        // (slice 2; it was `PropertyKey`, which now means a key that cannot
        // hold the new name), and every site carries its kind.
        for (text, org, old, to, reason, kind) in [
            (
                "* heading :Old:\n",
                true,
                "Old",
                "two words",
                HeadlineTag,
                K::Tag,
            ),
            (
                "- Old:: value\n",
                false,
                "Old",
                "Two Words",
                KeySpelling,
                K::PropertyKey,
            ),
            (
                "- {{query (page [[Old]])}}\n",
                false,
                "Old",
                "bad}name",
                StoredQuery,
                K::Query,
            ),
            (
                "- {{renderer [[Old]]}}\n",
                false,
                "Old",
                "bad}name",
                MacroLink,
                K::MacroArg,
            ),
            (
                "- {{query (page \"Old\n  name\")}}\n",
                false,
                "Old\nname",
                "New",
                CrossPiece,
                K::Query,
            ),
        ] {
            let map = HashMap::from([(crate::refs::page_key(old), to.to_owned())]);
            let p = crate::refs::rename_proposal(text, org, &map, FileNameFormat::TripleLowbar)
                .unwrap();
            assert!(
                p.unrealized
                    .iter()
                    .any(|s| s.reason == reason && s.kind == kind),
                "{reason:?} {kind:?}: {:?}",
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
            let plain = crate::refs::rename_rewrite_recording(
                &text,
                false,
                &map,
                FileNameFormat::TripleLowbar,
                None,
            );
            let walks = super::BLOCK_WALKS.with(|walks| walks.replace(0));
            let mut names = HashSet::new();
            let start = Instant::now();
            let recorded = crate::refs::rename_rewrite_recording(
                &text,
                false,
                &map,
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
            crate::refs::rename_rewrite_recording(
                text,
                false,
                &[("Old".into(), "Bad}name".into())],
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
