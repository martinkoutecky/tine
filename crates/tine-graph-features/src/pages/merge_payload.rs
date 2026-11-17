//! Physical merge realization: the survivor file the model's merge rule O6
//! describes (`cm-cut-model-r6.md`). Extends the reference span writer rather
//! than serializing either body's DTO. O(changed file bytes), no I/O; the
//! caller queues the checked bytes through PagePlan's guarded step.
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};
use tine_core::{
    model::{Format, PageDto},
    refs,
    text_trace::{Placement, Role, TextTrace, TraceError},
};
mod body;
use body::Body;

/// Provenance of each output block; a combined header can have two origins.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    TargetHeader,
    Target(usize),
    Leftover,
    Source(usize),
}

/// Checked physical survivor proposal. The trace, origins and unrealized sites
/// are ephemeral evidence for the CM checker, not fields of RenameReport.
#[derive(Clone, Debug)]
pub struct MergeProposal {
    pub text: String,
    pub trace: TextTrace,
    pub origins: Vec<Vec<Origin>>,
    pub unrealized: Vec<(usize, refs::Site)>,
    pub rewritten: HashSet<(String, String)>,
}

/// A merge refused before any write. `HeaderChildren`, `HiddenOccurrence`
/// (`leftoverReference`) and `SurvivorIdentity` are the model's refusals; the
/// others are the writer's checks that its bytes read as O6 says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RealizationError {
    Trace(TraceError),
    BodyLaw,
    IndentedBody,
    HeaderChildren,
    HiddenOccurrence,
    SurvivorIdentity,
    DirectiveEscape,
    FusedLineEnding,
    MissingNativeNameRange { declaration: String },
}
impl std::fmt::Display for RealizationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if matches!(self, Self::MissingNativeNameRange { .. }) {
            f.write_str("F-LINK")
        } else {
            write!(f, "merge realization: {self:?}")
        }
    }
}
impl std::error::Error for RealizationError {}
impl From<TraceError> for RealizationError {
    fn from(error: TraceError) -> Self {
        Self::Trace(error)
    }
}
/// Where realization failed: either native input or the assembled output.
/// Output failures name the survivor, whose proposed bytes failed the check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergePart {
    Survivor,
    Source,
    Output,
}
impl From<usize> for MergePart {
    fn from(file: usize) -> Self {
        if file == 0 {
            Self::Survivor
        } else {
            Self::Source
        }
    }
}

/// A realization reason with provenance captured at its raising site.
/// Error-only metadata; no extra reads or parsing on the successful path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RealizationFailure {
    pub part: MergePart,
    pub reason: RealizationError,
}
impl RealizationError {
    fn at(self, part: MergePart) -> RealizationFailure {
        RealizationFailure { part, reason: self }
    }
}
impl From<TraceError> for RealizationFailure {
    fn from(error: TraceError) -> Self {
        RealizationError::Trace(error).at(MergePart::Output)
    }
}
impl std::fmt::Display for RealizationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.reason.fmt(f)
    }
}
impl std::error::Error for RealizationFailure {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HeaderAt {
    PreBlock,
    FirstRoot,
}

pub(super) fn header_of(page: &PageDto) -> Option<(HeaderAt, &str)> {
    let first = page.blocks.first().map(|block| block.raw.as_str());
    let text = tine_store::query::page_header_text(page.pre_block.as_deref(), first)?;
    Some((
        if page.pre_block.is_some() {
            HeaderAt::PreBlock
        } else {
            HeaderAt::FirstRoot
        },
        text,
    ))
}

#[cfg(test)]
thread_local! {
    /// Drawer-ownership and delimiter comparisons, for the cost test.
    static DRAWER_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static BODY_LINES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static BODY_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[inline]
fn drawer_step() {
    #[cfg(test)]
    DRAWER_STEPS.with(|n| n.set(n.get() + 1));
}

struct Line {
    range: Range<usize>,
    content: Range<usize>,
    /// The key as the reader compares keys, after the name map (O6 compares
    /// keys after O1/O7), and the value.
    property: Option<(String, String)>,
    /// The property key's bytes.
    key: Range<usize>,
    /// The Org head drawer (index into `Header::drawers`) holding this line.
    drawer: Option<usize>,
    alias: Option<Alias>,
}
/// One alias declaration as the parser reads it. Its bytes are the key
/// (line content start through the parser's value start), the member tokens
/// (each holding its decoded name's bytes) and the syntax between them.
struct Alias {
    value: usize,
    names: Vec<(String, Range<usize>)>,
    tokens: Vec<Range<usize>>,
    movable: bool,
    /// Per member: kept (the first that reads as its name), else dropped.
    kept: Vec<bool>,
}
/// An alias declaration with members (O6 unites exactly these).
fn members(line: &Line) -> bool {
    line.alias.as_ref().is_some_and(|a| !a.names.is_empty())
}
struct Header {
    end: usize,
    body: Body,
    first_root: bool,
    properties: bool,
    prose: Option<Range<usize>>,
    lines: Vec<Line>,
    /// Each head drawer's `:PROPERTIES:` and `:END:` line indices.
    drawers: Vec<(usize, usize)>,
}

impl Header {
    fn read(page: &PageDto, text: &str, org: bool) -> Self {
        let blocks = refs::physical_outline(text, org);
        let first_root = header_of(page).is_some_and(|(at, _)| at == HeaderAt::FirstRoot);
        let end = if first_root {
            blocks.first().map_or(text.len(), |b| b.0.end)
        } else {
            blocks.first().map_or(text.len(), |b| b.0.start)
        };
        // The outline authority supplies structural prefixes. Classification
        // uses the same dedented header that the reader's property query sees.
        let indent = if first_root {
            blocks.first().map_or(0, |b| b.3)
        } else {
            0
        };
        let mut physical = 0;
        let mut logical = String::new();
        let mut lines = Vec::new();
        // Physical lines (`doc::physical_lines`); the classifier reads them in
        // the same-length view where a lone CR is LF, as the reader does.
        let view = tine_core::org::lone_cr_to_lf(&text[..end]);
        for raw in tine_core::doc::physical_lines(&text[..end]) {
            let skip = if first_root && blocks.first().is_some_and(|b| b.0.start == physical) {
                blocks.first().map_or(0, |b| b.1 - physical)
            } else if first_root && blocks.first().is_some_and(|b| physical > b.0.start) {
                // This is layout mapping, not property recognition.
                raw.bytes()
                    .take(indent)
                    .take_while(|c| matches!(c, b' ' | b'\t'))
                    .count()
            } else {
                0
            };
            let start = logical.len();
            logical.push_str(&view[physical + skip..physical + raw.len()]);
            lines.push((
                physical..physical + raw.len(),
                physical + skip..physical + raw.len(),
                start,
            ));
            physical += raw.len();
        }
        // The page's properties are the parser's answer, the same one the
        // reader and the final field check ask (`page_property_lines`):
        // Markdown `key::` lines, Org `#+KEY:` directives and head drawers.
        let regions = tine_core::block_regions::parse_document(&logical, org);
        let properties: HashMap<_, _> = regions.page_properties().map(|p| (p.line.0, p)).collect();
        let boxes = regions.property_drawer_boundaries(&logical);
        debug_assert!(boxes.windows(2).all(|w| w[0].1 < w[1].0));
        // Lines and drawers both run in source order: one forward cursor
        // assigns each line its drawer, O(lines + drawers).
        let mut cursor = 0;
        let (has_properties, prose) = page.pre_block.as_deref().map_or((first_root, None), |pre| {
            let (properties, prose) = tine_core::page_properties::pre_block_parts(pre, org);
            let prose = prose.map(|r| {
                // Both models are LF-canonical: each CRLF before the prose
                // is one byte the parser's offset does not count.
                let mut extra = 0;
                for (at, _) in text[..end].match_indices("\r\n") {
                    if at - extra >= r.start {
                        break;
                    }
                    extra += 1;
                }
                r.start + extra..end
            });
            (!properties.is_empty(), prose)
        });
        let mut lines: Vec<Line> = lines
            .into_iter()
            .filter(|(range, _, _)| prose.as_ref().is_none_or(|p| range.start < p.start))
            .map(|(range, content, at)| {
                let found = properties.get(&at);
                // Parser offsets are in the logical line; map them to bytes.
                let bytes = |r: &tine_core::block_regions::Range| {
                    content.start + r.0 - at..content.start + r.1 - at
                };
                let alias = found
                    .filter(|p| tine_core::page_header::is_alias_key(&p.key))
                    .map(|p| Alias::read(text, content.clone(), bytes(&p.value_range), org));
                Line {
                    // Keys as the reader compares them (`DocBlock::property`).
                    property: found
                        .map(|p| (tine_core::doc::property_key_norm(&p.key), p.value.clone())),
                    key: found.map_or(0..0, |p| bytes(&p.key_range)),
                    drawer: {
                        while boxes.get(cursor).is_some_and(|&(_, close)| close < at) {
                            drawer_step();
                            cursor += 1;
                        }
                        drawer_step();
                        boxes
                            .get(cursor)
                            .filter(|&&(open, _)| open <= at)
                            .map(|_| cursor)
                    },
                    range,
                    content,
                    alias,
                }
            })
            .collect();
        // Re-index by the drawers the header holds (a prose one is not).
        let mut drawers: Vec<(usize, usize)> = Vec::new();
        let mut index = HashMap::new();
        for (n, line) in lines.iter_mut().enumerate() {
            if let Some(d) = line.drawer {
                let at = *index.entry(d).or_insert_with(|| {
                    drawers.push((n, n));
                    drawers.len() - 1
                });
                drawers[at].1 = n;
                line.drawer = Some(at);
            }
        }
        Self {
            end,
            body: Body::read(text, &blocks, end, org),
            first_root,
            properties: has_properties,
            prose,
            lines,
            drawers,
        }
    }

    /// Whether line `n` opens or closes a head drawer: its own drawer's
    /// first or last line. O(1).
    fn delimiter(&self, n: usize) -> bool {
        drawer_step();
        self.lines[n]
            .drawer
            .is_some_and(|d| n == self.drawers[d].0 || n == self.drawers[d].1)
    }

    fn cuts(&self, text: &str) -> Vec<usize> {
        let mut cuts = vec![self.end];
        if let Some(prose) = &self.prose {
            cuts.push(prose.start);
            cuts.extend(tine_core::doc::physical_lines(&text[prose.clone()]).scan(
                prose.start,
                |at, l| {
                    *at += l.len();
                    Some(*at)
                },
            ));
        }
        for l in &self.lines {
            cuts.extend([l.range.start, l.content.start, l.content.end]);
            if let Some(a) = &l.alias {
                let raw = &text[l.content.clone()];
                cuts.extend([
                    a.value,
                    l.content.end - tine_core::doc::line_terminator(raw).len(),
                ]);
                for r in a.names.iter().map(|(_, r)| r).chain(&a.tokens) {
                    cuts.extend([r.start, r.end]);
                }
            }
        }
        cuts
    }
}

impl Alias {
    /// `range` is the line's content and `value` the parser's value range.
    fn read(text: &str, range: Range<usize>, value: Range<usize>, org: bool) -> Self {
        let raw = &text[range.clone()];
        let body = &raw[..raw.len() - tine_core::doc::line_terminator(raw).len()];
        let value_start = value.start - range.start;
        let mut at = value_start;
        let mut names = Vec::new();
        let mut tokens = Vec::new();
        let mut movable = true;
        // Reader-owned member parsing; never split a bracketed comma again.
        // A plain member's bytes are the parser's (escapes included); the
        // decoded name travels beside them for the reading comparison.
        tine_core::property_refs::for_each_linkable_member(
            &text[value],
            org,
            |name, token, bare| {
                let token = token.map(|r| value_start + r.start..value_start + r.end);
                let found = if bare {
                    token
                        .clone()
                        .filter(|r| at <= r.start && r.end <= body.len())
                } else {
                    body[at..].find(&name).map(|o| at + o..at + o + name.len())
                };
                let Some(Range { start, end }) = found else {
                    movable = false;
                    return;
                };
                let token = token
                    .filter(|r| {
                        at <= r.start && r.start <= start && end <= r.end && r.end <= body.len()
                    })
                    .unwrap_or(start..end);
                names.push((name, range.start + start..range.start + end));
                at = token.end;
                tokens.push(range.start + token.start..range.start + token.end);
            },
        );
        Self {
            value: range.start + value_start,
            names,
            tokens,
            movable,
            kept: Vec::new(),
        }
    }
}

struct Writer {
    trace: TextTrace,
    text: String,
    /// The terminator of every inserted line (see `propose`).
    eol: &'static str,
    /// Where the text's last placement ended, when it was original bytes.
    after: Option<(usize, usize)>,
    /// A CR that ended a line was followed by an LF it did not precede
    /// before: they would read as one CRLF, joining two lines.
    fused: bool,
}
/// Append placed bytes to the writer's text; `from` is the original piece's
/// (file, start, end). O(bytes).
fn place(
    text: &mut String,
    after: &mut Option<(usize, usize)>,
    fused: &mut bool,
    bytes: &str,
    from: Option<(usize, usize, usize)>,
) {
    let adjacent = from.is_some_and(|(file, start, _)| *after == Some((file, start)));
    *fused |= !adjacent && text.ends_with('\r') && bytes.starts_with('\n');
    *after = from.map(|(file, _, end)| (file, end));
    text.push_str(bytes);
}
impl Writer {
    fn interval(&self, file: usize, range: Range<usize>) -> Result<Vec<usize>, RealizationFailure> {
        self.trace
            .interval(file, range)
            .map_err(|error| RealizationError::Trace(error).at(file.into()))
    }
    /// The pre-block prose as its block. The empty lines that end the range
    /// separate the prose from the first block; they are not the block's
    /// text (the outline's `preamble_block` and the model check read the
    /// prose without them, manager ruling Q1), so they are layout.
    fn prose(
        &mut self,
        file: usize,
        range: Range<usize>,
        org: bool,
    ) -> Result<(), RealizationFailure> {
        self.insert(if org { "* " } else { "- " });
        let mut at = range.start;
        let lines: Vec<_> = tine_core::doc::physical_lines(&self.trace.inputs[file][range.clone()])
            .map(|line| (line.len(), line.trim_end_matches(['\r', '\n']).is_empty()))
            .collect();
        let content = lines
            .iter()
            .rposition(|&(_, blank)| !blank)
            .map_or(0, |n| n + 1);
        for (n, &(len, _)) in lines.iter().enumerate() {
            if n >= content {
                self.layout(file, at..at + len)?;
            } else {
                self.original(file, at..at + len)?;
                if n + 1 < content && !org {
                    self.insert("  ");
                }
            }
            at += len;
        }
        self.newline();
        Ok(())
    }
    fn value(&self, file: usize, range: Range<usize>) -> Result<String, RealizationFailure> {
        let mut text = String::new();
        for id in self.interval(file, range)? {
            text.push_str(
                self.trace
                    .substitution
                    .get(&id)
                    .map(String::as_str)
                    .unwrap_or_else(|| self.trace.original(id)),
            );
        }
        Ok(text)
    }
    /// The name the reader takes from a member as written: the parser's
    /// decoded `name`, or the name the rename wrote over its bytes.
    fn decoded(
        &self,
        file: usize,
        name: &str,
        range: Range<usize>,
    ) -> Result<String, RealizationFailure> {
        let ids = self.interval(file, range.clone())?;
        if ids
            .iter()
            .any(|id| self.trace.substitution.contains_key(id))
        {
            self.value(file, range)
        } else {
            Ok(name.to_owned())
        }
    }
    fn insert(&mut self, syntax: &str) {
        place(
            &mut self.text,
            &mut self.after,
            &mut self.fused,
            syntax,
            None,
        );
        self.trace.output.push(Placement::Insert(syntax.into()));
    }
    fn ids(&mut self, ids: impl IntoIterator<Item = usize>) {
        for id in ids {
            if self.trace.wrappers.contains(&id) {
                self.insert("[[");
            }
            let piece = &self.trace.pieces[id];
            place(
                &mut self.text,
                &mut self.after,
                &mut self.fused,
                self.trace
                    .substitution
                    .get(&id)
                    .map(String::as_str)
                    .unwrap_or_else(|| self.trace.original(id)),
                Some((piece.file, piece.range.start, piece.range.end)),
            );
            self.trace.output.push(Placement::Original(id));
            if self.trace.wrappers.contains(&id) {
                self.insert("]]");
            }
        }
    }
    fn original(&mut self, file: usize, range: Range<usize>) -> Result<(), RealizationFailure> {
        self.ids(self.interval(file, range)?);
        Ok(())
    }
    fn newline(&mut self) {
        if !self.text.is_empty() && tine_core::doc::line_terminator(&self.text).is_empty() {
            self.insert(self.eol);
        }
    }
    fn consume(&mut self, file: usize, range: Range<usize>) -> Result<(), RealizationFailure> {
        let ids = self.interval(file, range)?;
        self.trace.consumed.extend(ids);
        Ok(())
    }
    /// Drop bytes the parser does not report as content (P4: blank lines,
    /// structural prefixes, drawer frames); the trace checks each one.
    fn layout(&mut self, file: usize, range: Range<usize>) -> Result<(), RealizationFailure> {
        for id in self.interval(file, range)? {
            self.trace.pieces[id].role = Role::Layout;
            self.trace.consumed.push(id);
        }
        Ok(())
    }
    /// The united alias declaration, written on the retained `line` (O6): its
    /// key and first member as written (the first member is always kept),
    /// each later kept member with its own delimiter (`, ` where a dropped
    /// one stood before it), then `additions`, the kept member tokens of the
    /// other declarations. A dropped member and its syntax are consumed.
    fn field(
        &mut self,
        file: usize,
        line: &Line,
        additions: &[Vec<usize>],
    ) -> Result<(), RealizationFailure> {
        let a = line.alias.as_ref().unwrap();
        let raw = &self.trace.inputs[file][line.content.clone()];
        let end = line.content.end - tine_core::doc::line_terminator(raw).len();
        let mut at = line.content.start;
        let mut previous = true;
        for (token, &kept) in a.tokens.iter().zip(&a.kept) {
            if kept && !previous {
                self.insert(", ");
                self.consume(file, at..token.start)?;
                at = token.start;
            }
            if kept {
                self.original(file, at..token.end)?;
            } else {
                self.consume(file, at..token.end)?;
            }
            at = token.end;
            previous = kept;
        }
        self.original(file, at..end)?;
        for ids in additions {
            self.insert(", ");
            self.ids(ids.iter().copied());
        }
        self.original(file, end..line.content.end)
    }
}

/// How the reader identifies the page an alias member names: a name that
/// parses as a journal day reads as that day, any other by its page key
/// (the model's `Date13.target`, then `page_key`). O(name bytes).
fn reading(name: &str, format: &refs::RenameFormat<'_>) -> String {
    match format.days.and_then(|days| days.parse(name)) {
        Some(day) => format!("\u{0}{}", day.ordinal_key()),
        None => refs::page_key(name),
    }
}

/// Realize a merge from the exact bytes guarded by the caller's revisions.
/// `links` and `queries` are the span writer's already-keyed substitutions.
/// The survivor keeps its header and blocks; the source header is allocated
/// as O6 says, in input order. Failure returns no writable proposal.
/// O(changed bytes), no graph scan/I/O.
pub fn propose<'f>(
    target: &PageDto,
    target_text: &str,
    source: &PageDto,
    source_text: &str,
    links: &HashMap<String, String>,
    queries: &HashMap<String, String>,
    org: bool,
    format: impl Into<refs::RenameFormat<'f>>,
) -> Result<MergeProposal, RealizationFailure> {
    let format = format.into();
    let mut headers = [
        Header::read(target, target_text, org),
        Header::read(source, source_text, org),
    ];
    for (file, header) in headers.iter().enumerate() {
        if let Some(line) = header
            .lines
            .iter()
            .find(|line| line.alias.as_ref().is_some_and(|a| !a.movable))
        {
            // The reader decodes some legal spellings without native inner-name
            // coordinates. Do not lose or duplicate those alias members.
            let text = [target_text, source_text][file];
            return Err(RealizationError::MissingNativeNameRange {
                declaration: text[line.content.clone()].trim_end().into(),
            }
            .at(file.into()));
        }
    }
    // A childless first-root header would promote on the next ordinary save:
    // realize it as a native preamble now (P4: header position is free).
    let normalize_target = !org && headers[0].first_root && target.blocks[0].children.is_empty();
    // Org omits all final line breaks from its last block's content. Keep
    // one original terminator at the join and consume only the remaining
    // unobserved breaks as layout (O6/P4).
    let target_end = if org {
        let end = target_text.trim_end_matches(['\r', '\n']).len();
        let ending = tine_core::doc::physical_lines(&target_text[end..])
            .next()
            .map_or(0, str::len);
        (end + ending).max(headers[0].end)
    } else {
        target_text.len()
    };
    let mut trace = TextTrace::default();
    let mut unrealized = Vec::new();
    let mut rewritten = HashSet::new();
    let fmt = if org { Format::Org } else { Format::Md };
    for (file, text) in [target_text, source_text].into_iter().enumerate() {
        let mut input = || -> Result<(), RealizationError> {
            let mut proposal = refs::rename_proposal(text, org, links, queries, format)?;
            // O1 maps the file's own title too (`mapAtom`/`titleParts`), through
            // its one producer, before O6 allocates: a source title that becomes
            // a leftover line is written as the mapped title.
            if let Some(mapped) = tine_core::model::page_title_from_header(text, fmt)
                .and_then(|title| refs::renamed_page_name(&title, links).map(|n| n.into_owned()))
            {
                proposal.rebind_title(org, &mapped)?;
            }
            let edits: Vec<_> = proposal
                .trace
                .pieces
                .iter()
                .enumerate()
                .filter_map(|(id, p)| {
                    proposal
                        .trace
                        .substitution
                        .get(&id)
                        .map(|s| (p.range.clone(), s.clone()))
                })
                .collect();
            if file == 1 {
                let mut edit = 0;
                for prefix in &headers[1].body.prefixes {
                    while edit < edits.len() && edits[edit].0.end <= prefix.start {
                        edit += 1;
                    }
                    if edit < edits.len() && edits[edit].0.start < prefix.end {
                        return Err(RealizationError::IndentedBody);
                    }
                }
            }
            // O6 compares keys after O1/O7 (`mapHeader`): a key site the map
            // rewrites is one edit over the key's bytes (`explicit_property_key`),
            // and the mapped key is what the parser reads from its replacement.
            let rewritten_keys: HashMap<_, _> = edits.iter().map(|(r, s)| (r.clone(), s)).collect();
            for line in &mut headers[file].lines {
                if let Some((key, _)) = &mut line.property {
                    if let Some(to) = rewritten_keys.get(&line.key) {
                        *key = tine_core::doc::property_key_norm(to);
                    }
                }
            }
            let start = trace.pieces.len();
            let mut cuts = headers[file].cuts(text);
            cuts.extend(headers[file].body.cuts());
            if file == 0 {
                cuts.push(target_end);
            }
            trace.add_input(text, &edits, &cuts)?;
            let wrappers: HashSet<_> = proposal
                .trace
                .wrappers
                .iter()
                .map(|id| proposal.trace.pieces[*id].range.clone())
                .collect();
            trace.wrappers.extend(
                (start..trace.pieces.len())
                    .filter(|id| wrappers.contains(&trace.pieces[*id].range)),
            );
            trace.scope.insert(file);
            // A declaration with members: its key, names and syntax between them.
            for line in headers[file].lines.iter().filter(|l| members(l)) {
                let a = line.alias.as_ref().unwrap();
                let mut names = a.names.iter().map(|(_, r)| r).peekable();
                for id in trace.interval(file, line.content.clone())? {
                    let r = trace.pieces[id].range.clone();
                    while names.peek().is_some_and(|n| n.end <= r.start) {
                        names.next();
                    }
                    trace.pieces[id].role = if r.end <= a.value {
                        Role::AliasKey
                    } else if names.peek().is_some_and(|n| n.start <= r.start) {
                        Role::AliasName
                    } else {
                        Role::AliasDelimiter
                    };
                }
            }
            unrealized.extend(proposal.unrealized.into_iter().map(|site| (file, site)));
            rewritten.extend(proposal.rewritten);
            Ok(())
        };
        input().map_err(|reason| reason.at(file.into()))?;
    }
    let mut w = Writer {
        trace,
        text: String::new(),
        eol: tine_core::doc::line_convention(target_text),
        after: None,
        fused: false,
    };
    // O6: every alias declaration with members, of both files, becomes one,
    // holding each member once, survivor's first; a member that reads as a
    // kept one is dropped. It is written on the first such declaration (the
    // survivor's when it has one); the others' kept tokens join it there.
    let retained = (0..2).find_map(|file| {
        headers[file]
            .lines
            .iter()
            .position(members)
            .map(|n| (file, n))
    });
    let mut seen = HashSet::new();
    let mut additions = Vec::new();
    for file in 0..2 {
        for (n, line) in headers[file].lines.iter_mut().enumerate() {
            let Some(a) = &mut line.alias else { continue };
            for ((decoded, range), token) in a.names.iter().zip(&a.tokens) {
                let name = w.decoded(file, decoded, range.clone())?;
                let read = reading(&name, &format);
                for id in w.interval(file, range.clone())? {
                    w.trace.readings.insert(id, read.clone());
                }
                let kept = seen.insert(read);
                if kept && retained != Some((file, n)) {
                    additions.push(w.interval(file, token.clone())?);
                }
                a.kept.push(kept);
            }
        }
    }
    let relocated: HashSet<usize> = additions.iter().flatten().copied().collect();
    for file in 0..2 {
        for (n, line) in headers[file].lines.iter().enumerate() {
            if members(line) && retained != Some((file, n)) {
                for id in w.interval(file, line.content.clone())? {
                    if !relocated.contains(&id) {
                        w.trace.consumed.push(id);
                    }
                }
            }
        }
    }
    // A combined declaration's bytes outside its kept members must be alias
    // syntax. In scope: malformed input (other text inside an alias value,
    // `alias:: a **b**`), which dropping the syntax would lose.
    if let Some(&id) = w.trace.consumed.iter().find(|&&id| {
        let text = w.trace.original(id);
        match w.trace.pieces[id].role {
            Role::AliasKey => !tine_core::text_trace::alias_key(text),
            Role::AliasDelimiter => !tine_core::text_trace::alias_syntax(text),
            _ => false,
        }
    }) {
        let piece = &w.trace.pieces[id];
        let line = headers[piece.file]
            .lines
            .iter()
            .find(|line| {
                line.content.start <= piece.range.start && piece.range.end <= line.content.end
            })
            .expect("consumed alias syntax belongs to its header line");
        return Err(RealizationError::MissingNativeNameRange {
            declaration: w.trace.inputs[piece.file][line.content.clone()]
                .trim_end()
                .into(),
        }
        .at(piece.file.into()));
    }
    // O6, the other source header entries in order: a property whose key
    // (as the reader compares keys) neither the survivor nor an earlier
    // joined entry has joins; a title, a clashing or repeated property and
    // other header text are leftover lines. Blank lines, drawer frames and
    // structural prefixes are not content (P4).
    let mut keys: HashSet<_> = headers[0]
        .lines
        .iter()
        .filter_map(|l| l.property.as_ref().map(|p| p.0.clone()))
        .collect();
    let mut joined = Vec::new();
    let mut leftover_lines = Vec::new();
    for (n, line) in headers[1].lines.iter().enumerate() {
        let content = &source_text[line.content.clone()];
        let frame = tine_core::text_trace::blank(content) || headers[1].delimiter(n);
        if members(line) {
            if retained == Some((1, n)) {
                joined.push(n);
            }
        } else if frame {
        } else if line.alias.is_none()
            && line
                .property
                .as_ref()
                .is_some_and(|(key, _)| key != "title" && keys.insert(key.clone()))
        {
            joined.push(n);
        } else {
            leftover_lines.push(n);
        }
        w.layout(
            1,
            if frame && !members(line) {
                line.range.clone()
            } else {
                line.range.start..line.content.start
            },
        )?;
    }
    let leftover = !leftover_lines.is_empty();
    let source_header_children = if headers[1].first_root {
        source.blocks[0].children.as_slice()
    } else {
        &[]
    };
    if !leftover && !source_header_children.is_empty() {
        // O6 refusal `headerChildren`: blocks under the source's header hang
        // under the leftover block, and with no leftover lines there is none.
        return Err(RealizationError::HeaderChildren.at(MergePart::Source));
    }
    // Joining drawer entries go into one head drawer: the target's last, else
    // a new one placed first. One drawer reads the same in Logseq (mldoc
    // folds a drawer and the directives after it into one; a directive before
    // a drawer hides it) and in Tine for every line ending.
    let boxed: Vec<_> = joined
        .iter()
        .copied()
        .filter(|&n| headers[1].lines[n].drawer.is_some())
        .collect();
    let promoted: Vec<_> = joined
        .iter()
        .copied()
        .filter(|&n| headers[1].lines[n].drawer.is_none())
        .collect();
    let target_close = headers[0].drawers.last().map(|d| d.1);
    let new_drawer = target_close.is_none() && !boxed.is_empty();
    let header_insert = headers[0]
        .lines
        .iter()
        .rposition(|l| !target_text[l.range.clone()].trim().is_empty())
        .map_or(0, |n| n + 1);
    // A target header's structural prefixes are positional: its j-th output
    // line takes its j-th prefix (the first is the block's bullet), whichever
    // line's content lands there. Prefixes left over are layout; a preamble
    // header has none, a normalized one keeps none.
    let emitted = |n: usize, line: &Line| !members(line) || retained == Some((0, n));
    let slots: Vec<_> = headers[0].lines[..header_insert]
        .iter()
        .map(|line| line.range.start..line.content.start)
        .filter(|r| !r.is_empty())
        .collect();
    let used = if normalize_target {
        0
    } else {
        let lines = headers[0].lines[..header_insert].iter().enumerate();
        slots
            .len()
            .min(lines.filter(|(n, l)| emitted(*n, l)).count() + joined.len())
    };
    for r in &slots[used..] {
        w.layout(0, r.clone())?;
    }
    let mut slots = slots[..used].iter();
    let entries = |w: &mut Writer| -> Result<(), RealizationFailure> {
        for &n in &boxed {
            w.newline();
            let line = &headers[1].lines[n];
            if retained == Some((1, n)) {
                w.field(1, line, &additions)?;
            } else {
                w.original(1, line.content.clone())?;
            }
        }
        Ok(())
    };
    let source_drawer = |w: &mut Writer| -> Result<(), RealizationFailure> {
        if new_drawer {
            w.insert(&format!(":PROPERTIES:{}", w.eol));
            entries(w)?;
            w.newline();
            w.insert(&format!(":END:{}", w.eol));
            w.newline();
        }
        Ok(())
    };
    for (n, line) in headers[0].lines[..header_insert].iter().enumerate() {
        if n == 0 {
            source_drawer(&mut w)?;
        }
        if target_close == Some(n) {
            entries(&mut w)?;
            w.newline();
        }
        if !emitted(n, line) {
            continue;
        }
        if let Some(prefix) = slots.next() {
            w.original(0, prefix.clone())?;
        }
        if retained == Some((0, n)) {
            w.field(0, line, &additions)?;
        } else {
            w.original(0, line.content.clone())?;
        }
    }
    if header_insert == 0 {
        source_drawer(&mut w)?;
    }
    for &n in &promoted {
        let line = &headers[1].lines[n];
        w.newline();
        if let Some(prefix) = slots.next() {
            w.original(0, prefix.clone())?;
        } else if headers[0].first_root && !normalize_target {
            w.insert("  ");
        }
        if retained == Some((1, n)) {
            w.field(1, line, &additions)?;
        } else {
            w.original(1, line.content.clone())?;
        }
    }
    w.newline();
    for line in &headers[0].lines[header_insert..] {
        w.original(0, line.range.clone())?;
    }
    if let Some(prose) = &headers[0].prose {
        w.prose(0, prose.clone(), org)?;
    }
    w.original(0, headers[0].end..target_end)?;
    w.newline();
    // O6's leftover block, top level after the survivor's blocks: an empty
    // first line (Org: a headline with an empty title), then the leftover
    // lines, in Org inside a quote, which owns them as body text (OG groups
    // every top-level directive, even below a headline: og 6e7afa8,
    // graph-parser/mldoc.cljc).
    let leftover_start = w.text.len();
    if leftover {
        let eol = w.eol;
        if org {
            w.insert(&format!("*{eol}"));
            w.insert(&format!("#+BEGIN_QUOTE{eol}"));
        } else {
            w.insert("-");
            w.insert(eol);
        }
        for &n in &leftover_lines {
            if !org {
                w.insert("  ");
            }
            w.original(1, headers[1].lines[n].content.clone())?;
            w.newline();
        }
        if org {
            w.insert(&format!("#+END_QUOTE{eol}"));
        }
    }
    let leftover_end = w.text.len();
    if let Some(prose) = &headers[1].prose {
        w.prose(1, prose.clone(), org)?;
    }
    let mut at = headers[1].end;
    for prefix in &headers[1].body.prefixes {
        w.original(1, at..prefix.start)?;
        w.layout(1, prefix.clone())?;
        at = prefix.end;
    }
    w.original(1, at..source_text.len())?;
    w.layout(0, target_end..target_text.len())?;
    w.trace.check(&w.text, &w.trace.substitution)?;
    if w.fused {
        // In scope: a mixed-ending page (external editors, sync). No
        // convention-following terminator can separate the lines here.
        return Err(RealizationError::FusedLineEnding.at(MergePart::Output));
    }
    if org && leftover {
        // A leftover line must stay inside its quote (a line reading
        // `#+END_QUOTE` would end it): leftover lines are not page properties.
        let quote = tine_core::org::lone_cr_to_lf(&w.text[leftover_start..leftover_end]);
        let ast = tine_core::render::parse_text_bounded(&quote, true)
            .ok_or_else(|| RealizationError::DirectiveEscape.at(MergePart::Source))?;
        if ast
            .blocks
            .iter()
            .any(|block| matches!(block, tine_core::lsdoc::ast::Block::Directive { .. }))
        {
            return Err(RealizationError::DirectiveEscape.at(MergePart::Source));
        }
    }
    let read = if org {
        tine_core::org::parse_org(&w.text)
    } else {
        tine_core::doc::parse(&w.text)
    };
    if tine_core::page_header::page_header_text(
        read.pre_block.as_deref(),
        read.roots.first().map(|block| block.raw()),
    )
    .and_then(|header| tine_core::model::page_title_from_header(header, fmt))
    .is_some_and(|name| {
        !refs::same_page(&name, &target.name)
            && !format.days.is_some_and(|days| {
                days.parse(&name)
                    .is_some_and(|day| days.parse(&target.name) == Some(day))
            })
    }) {
        // The survivor keeps its name (O6, P3): a header the save would read
        // as another title is refused, not patched.
        return Err(RealizationError::SurvivorIdentity.at(MergePart::Output));
    }
    // Whether the survivor reads the united alias members in one declaration
    // is the transaction's model check (P2), on the final bytes.
    let source_body = &source.blocks[usize::from(headers[1].first_root)..];
    let skip = usize::from(headers[0].first_root && !normalize_target);
    let roots = &read.roots;
    let target_prose = usize::from(headers[0].prose.is_some());
    let source_prose = usize::from(headers[1].prose.is_some());
    let target_roots = target.blocks.len() - usize::from(normalize_target) + target_prose;
    let source_start = target_roots + usize::from(leftover);
    let count = source_start + source_prose + source_body.len();
    if roots.len() != count {
        let source_indented = !headers[1].body.prefixes.is_empty();
        return Err(if source_indented {
            RealizationError::IndentedBody.at(MergePart::Source)
        } else {
            RealizationError::BodyLaw.at(MergePart::Output)
        });
    }
    body::check_payloads(
        &w,
        0,
        &headers[0],
        (target_prose > 0).then(|| &roots[0]),
        None,
        org,
    )?;
    body::check_payloads(
        &w,
        1,
        &headers[1],
        (source_prose > 0).then(|| &roots[source_start]),
        leftover.then(|| (leftover_lines.as_slice(), &roots[target_roots])),
        org,
    )?;
    let source_child_count = headers[1].body.check(
        &w,
        1,
        source_header_children,
        if leftover {
            &roots[target_roots].children
        } else {
            &[]
        },
        usize::from(headers[1].first_root),
        false,
    )?;
    let target_body_count = headers[0].body.check(
        &w,
        0,
        &target.blocks[usize::from(normalize_target)..],
        &roots[target_prose..target_roots],
        usize::from(normalize_target),
        headers[0].first_root && !normalize_target,
    )? - skip;
    let source_body_count = headers[1].body.check(
        &w,
        1,
        source_body,
        &roots[source_start + source_prose..],
        usize::from(headers[1].first_root) + source_child_count,
        false,
    )?;
    if org && leftover {
        // O6 refusal `leftoverReference` (model `leftoverUnkeepable`): the
        // references the leftover lines had, read where they came from (a
        // drawer entry inside its drawer), are exactly the ones Tine reads
        // in the quote, kind and name.
        let mut required = HashMap::new();
        let count_refs =
            |block: &tine_core::DocBlock, counts: &mut HashMap<(&'static str, String), usize>| {
                for r in block.projection().reference_source().explicit {
                    if matches!(r.rule, "explicit_task_marker" | "explicit_priority") {
                        continue;
                    }
                    *counts.entry((r.rule, refs::page_key(&r.name))).or_default() += 1;
                }
            };
        let header = &headers[1];
        for &n in &leftover_lines {
            let line = &header.lines[n];
            let mut original = String::new();
            let drawer = line.drawer.map(|d| header.drawers[d]);
            if let Some((open, _)) = drawer {
                original.push_str(&w.value(1, header.lines[open].content.clone())?);
            }
            original.push_str(&w.value(1, line.content.clone())?);
            if tine_core::doc::line_terminator(&original).is_empty() {
                original.push('\n');
            }
            if let Some((_, close)) = drawer {
                original.push_str(&w.value(1, header.lines[close].content.clone())?);
                if tine_core::doc::line_terminator(&original).is_empty() {
                    original.push('\n');
                }
            }
            let original = tine_core::org::lone_cr_to_lf(&original);
            let mut own = HashMap::new();
            for (_, block) in tine_core::page_properties::pre_block_projections(&original, true) {
                count_refs(&block, &mut own);
            }
            for (site, n) in own {
                *required.entry(site).or_default() += n;
            }
        }
        let mut actual = HashMap::new();
        count_refs(&roots[target_roots], &mut actual);
        if required != actual {
            return Err(RealizationError::HiddenOccurrence.at(MergePart::Source));
        }
    }
    let source_header = usize::from(headers[1].properties) + source_prose;
    let target_header = usize::from(headers[0].properties) + target_prose;
    let target_property_origin = || {
        if target_prose == 0 {
            Origin::TargetHeader
        } else {
            Origin::Target(0)
        }
    };
    let has_promoted = !joined.is_empty() || !additions.is_empty();
    let mut origins = Vec::new();
    for _ in 0..usize::from(read.pre_block.is_some()) + skip {
        let mut origin = Vec::new();
        if headers[0].properties {
            origin.push(target_property_origin());
        }
        if has_promoted && headers[1].properties {
            origin.push(Origin::Source(0));
        }
        origins.push(origin);
    }
    if target_prose > 0 {
        origins.push(vec![Origin::Target(usize::from(headers[0].properties))]);
    }
    origins.extend((0..target_body_count).map(|j| vec![Origin::Target(target_header + j)]));
    if leftover {
        let mut origin = vec![Origin::Leftover];
        if headers[1].properties {
            origin.push(Origin::Source(0));
        }
        origins.push(origin);
        origins.extend((0..source_child_count).map(|j| vec![Origin::Source(source_header + j)]));
    }
    if source_prose > 0 {
        origins.push(vec![Origin::Source(usize::from(headers[1].properties))]);
    }
    origins.extend(
        (0..source_body_count)
            .map(|j| vec![Origin::Source(source_header + source_child_count + j)]),
    );
    Ok(MergeProposal {
        text: w.text,
        trace: w.trace,
        origins,
        unrealized,
        rewritten,
    })
}

#[cfg(test)]
#[path = "merge_payload_tests.rs"]
mod tests;
