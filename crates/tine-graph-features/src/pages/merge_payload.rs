//! Physical merge realization (SPEC §5.1–5.2). Extends the reference span
//! writer rather than serializing either body's DTO. O(changed file bytes),
//! no I/O; the caller queues the checked bytes through PagePlan's guarded step.
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};
use tine_core::{
    model::{BlockDto, Format, PageDto},
    refs,
    text_trace::{Placement, Role, TextTrace, TraceError},
};

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

/// A candidate whose bytes cannot realize the specified merge layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RealizationError {
    Trace(TraceError),
    BodyLaw,
    HiddenOccurrence,
    SurvivorIdentity,
    AliasField,
    DirectiveEscape,
}
impl std::fmt::Display for RealizationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "merge realization: {self:?}")
    }
}
impl std::error::Error for RealizationError {}
impl From<TraceError> for RealizationError {
    fn from(e: TraceError) -> Self {
        Self::Trace(e)
    }
}

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

struct Line {
    range: Range<usize>,
    content: Range<usize>,
    property: Option<(String, String)>,
    alias: Option<Alias>,
}
struct Alias {
    key: Option<Range<usize>>,
    names: Vec<(String, Range<usize>)>,
    tokens: Vec<Range<usize>>,
    syntax: Vec<Range<usize>>,
    plain: bool,
    movable: bool,
}
struct Header {
    end: usize,
    first_root: bool,
    properties: bool,
    prose: Option<Range<usize>>,
    lines: Vec<Line>,
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
        for raw in text[..end].split_inclusive('\n') {
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
            logical.push_str(&raw[skip..]);
            lines.push((
                physical..physical + raw.len(),
                physical + skip..physical + raw.len(),
                start,
            ));
            physical += raw.len();
        }
        let fmt = if org { Format::Org } else { Format::Md };
        let regions = crate::conflicts::pre_regions(&logical, fmt);
        let (properties, prose) = page.pre_block.as_deref().map_or((first_root, None), |pre| {
            let (properties, prose) = tine_core::page_properties::pre_block_parts(pre, org);
            let prose = prose.map(|r| {
                let mut extra = 0;
                if !org {
                    for (at, _) in text[..end].match_indices("\r\n") {
                        if at - extra >= r.start {
                            break;
                        }
                        extra += 1;
                    }
                }
                r.start + extra..end
            });
            (!properties.is_empty(), prose)
        });
        let lines = lines
            .into_iter()
            .filter(|(range, _, _)| prose.as_ref().is_none_or(|p| range.start < p.start))
            .map(|(range, content, at)| {
                let property = regions.property(at).cloned();
                let alias = property
                    .as_ref()
                    .filter(|(k, _)| tine_store::query::is_alias_key(k))
                    .map(|(_, value)| Alias::read(text, content.clone(), value, org));
                Line {
                    range,
                    content,
                    property,
                    alias,
                }
            })
            .collect();
        Self {
            end,
            first_root,
            properties,
            prose,
            lines,
        }
    }

    fn cuts(&self, text: &str) -> Vec<usize> {
        let mut cuts = vec![self.end];
        if let Some(prose) = &self.prose {
            cuts.push(prose.start);
            cuts.extend(
                text[prose.clone()]
                    .match_indices('\n')
                    .map(|(at, _)| prose.start + at + 1),
            );
        }
        for l in &self.lines {
            cuts.extend([l.range.start, l.content.start, l.content.end]);
            if let Some(a) = &l.alias {
                for r in a
                    .key
                    .iter()
                    .chain(a.names.iter().map(|(_, r)| r))
                    .chain(&a.tokens)
                    .chain(&a.syntax)
                {
                    cuts.extend([r.start, r.end]);
                }
            }
        }
        cuts
    }
}

impl Alias {
    fn read(text: &str, range: Range<usize>, value: &str, org: bool) -> Self {
        let raw = &text[range.clone()];
        let body = raw
            .strip_suffix("\r\n")
            .or_else(|| raw.strip_suffix('\n'))
            .unwrap_or(raw);
        let key = ["alias:: ", "aliases:: ", "#+alias: ", "#+aliases: "]
            .into_iter()
            .find(|key| body.starts_with(key))
            .map(|key| range.start..range.start + key.len());
        let mut at = key.as_ref().map_or_else(
            || body.find(value).unwrap_or(body.len()),
            |r| r.end - range.start,
        );
        let mut names = Vec::new();
        let mut tokens = Vec::new();
        let mut syntax = Vec::new();
        let mut plain = key.is_some();
        let mut movable = true;
        let value_start = body.find(value).unwrap_or(at);
        // Reader-owned member parsing; never split a bracketed comma again.
        tine_core::property_refs::for_each_linkable_member(value, org, |name, token| {
            let Some(offset) = body[at..].find(&name) else {
                plain = false;
                movable = false;
                return;
            };
            let start = at + offset;
            let end = start + name.len();
            let token = token
                .map(|r| value_start + r.start..value_start + r.end)
                .filter(|r| {
                    at <= r.start && r.start <= start && end <= r.end && r.end <= body.len()
                })
                .unwrap_or(start..end);
            plain &= token == (start..end);
            if token.start > at {
                let gap = at..token.start;
                if matches!(&body[gap.clone()], ", " | ",") {
                    syntax.push(range.start + gap.start..range.start + gap.end);
                } else {
                    plain = false;
                }
            }
            names.push((name, range.start + start..range.start + end));
            at = token.end;
            tokens.push(range.start + token.start..range.start + token.end);
        });
        if at != body.len() {
            plain = false;
        }
        if body.len() < raw.len() {
            syntax.push(range.start + body.len()..range.end);
        }
        Self {
            key,
            names,
            tokens,
            syntax,
            plain,
            movable,
        }
    }
}

struct Writer {
    trace: TextTrace,
    text: String,
}
impl Writer {
    fn prose(&mut self, file: usize, range: Range<usize>, org: bool) -> Result<(), TraceError> {
        self.insert(if org { "* " } else { "- " });
        let mut at = range.start;
        while at < range.end {
            let end = self.trace.inputs[file][at..range.end]
                .find('\n')
                .map_or(range.end, |n| at + n + 1);
            self.original(file, at..end)?;
            if end < range.end && !org {
                self.insert("  ");
            }
            at = end;
        }
        self.newline();
        Ok(())
    }
    fn value(&self, file: usize, range: Range<usize>) -> Result<String, TraceError> {
        let mut text = String::new();
        for id in self.trace.interval(file, range)? {
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
    fn insert(&mut self, syntax: &str) {
        self.text.push_str(syntax);
        self.trace.output.push(Placement::Insert(syntax.into()));
    }
    fn ids(&mut self, ids: impl IntoIterator<Item = usize>) {
        for id in ids {
            if self.trace.wrappers.contains(&id) {
                self.insert("[[");
            }
            self.text.push_str(
                self.trace
                    .substitution
                    .get(&id)
                    .map(String::as_str)
                    .unwrap_or_else(|| self.trace.original(id)),
            );
            self.trace.output.push(Placement::Original(id));
            if self.trace.wrappers.contains(&id) {
                self.insert("]]");
            }
        }
    }
    fn original(&mut self, file: usize, range: Range<usize>) -> Result<(), TraceError> {
        self.ids(self.trace.interval(file, range)?);
        Ok(())
    }
    fn newline(&mut self) {
        if !self.text.is_empty() && !self.text.ends_with('\n') {
            self.insert("\n");
        }
    }
    fn field(
        &mut self,
        file: usize,
        line: &Line,
        additions: &[Vec<Placement>],
        prefix: bool,
    ) -> Result<(), TraceError> {
        let a = line.alias.as_ref().unwrap();
        let start = if prefix {
            line.range.start
        } else {
            line.content.start
        };
        let raw = &self.trace.inputs[file][line.content.clone()];
        let ending = if raw.ends_with("\r\n") {
            2
        } else {
            usize::from(raw.ends_with('\n'))
        };
        let end = line.content.end - ending;
        if a.plain {
            let key = a.key.as_ref().unwrap();
            self.original(file, start..key.end)?;
            let mut previous = key.end;
            let mut seen = HashSet::new();
            for (_, range) in &a.names {
                if seen.insert(self.value(file, range.clone())?) {
                    self.original(file, previous..range.start)?;
                    self.original(file, range.clone())?;
                } else {
                    self.trace
                        .consumed
                        .extend(self.trace.interval(file, previous..range.end)?);
                }
                previous = range.end;
            }
            self.original(file, previous..end)?;
        } else {
            self.original(file, start..end)?;
        }
        for (n, items) in additions.iter().enumerate() {
            if n > 0 || !a.names.is_empty() {
                self.insert(", ");
            }
            for p in items {
                match p {
                    Placement::Original(id) => self.ids([*id]),
                    Placement::Insert(s) => self.insert(s),
                }
            }
        }
        self.original(file, end..line.content.end)?;
        Ok(())
    }
}

/// Realize a merge from the exact bytes guarded by the caller's revisions.
/// `links` and `queries` are the span writer's already-keyed substitutions.
/// Failure returns no writable proposal. O(changed bytes), no graph scan/I/O.
pub fn propose(
    target: &PageDto,
    target_text: &str,
    source: &PageDto,
    source_text: &str,
    links: &HashMap<String, String>,
    queries: &HashMap<String, String>,
    org: bool,
    format: tine_core::config::FileNameFormat,
) -> Result<MergeProposal, RealizationError> {
    let headers = [
        Header::read(target, target_text, org),
        Header::read(source, source_text, org),
    ];
    // A childless first-root header would promote on the next ordinary save.
    // Realize it as a native preamble now; retain its old outline prefix in
    // the same inert leftover as other protected header syntax. A header with
    // children is already nonpromoting and keeps its body parent.
    let normalize_target = !org && headers[0].first_root && target.blocks[0].children.is_empty();
    let mut trace = TextTrace::default();
    let mut unrealized = Vec::new();
    let mut rewritten = HashSet::new();
    for (file, text) in [target_text, source_text].into_iter().enumerate() {
        let proposal = refs::rename_proposal(text, org, links, queries, format)?;
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
        let start = trace.pieces.len();
        trace.add_input(text, &edits, &headers[file].cuts(text))?;
        let wrappers: HashSet<_> = proposal
            .trace
            .wrappers
            .iter()
            .map(|id| proposal.trace.pieces[*id].range.clone())
            .collect();
        trace.wrappers.extend(
            (start..trace.pieces.len()).filter(|id| wrappers.contains(&trace.pieces[*id].range)),
        );
        trace.scope.insert(file);
        for line in &headers[file].lines {
            if let Some(a) = &line.alias {
                for (range, role) in a
                    .key
                    .iter()
                    .map(|r| (r, Role::AliasKey))
                    .chain(a.syntax.iter().map(|r| (r, Role::AliasDelimiter)))
                    .chain(a.names.iter().map(|(_, r)| (r, Role::AliasName)))
                {
                    for id in trace.interval(file, range.clone())? {
                        trace.pieces[id].role = role.clone();
                    }
                }
            }
        }
        unrealized.extend(proposal.unrealized.into_iter().map(|site| (file, site)));
        rewritten.extend(proposal.rewritten);
    }
    // The retained declaration is the first survivor alias, otherwise the
    // first source declaration. Other source fields become protected leftover
    // text unless their entire line is approved native syntax/name tokens.
    let retained = (0..2).find_map(|file| {
        headers[file]
            .lines
            .iter()
            .position(|line| line.alias.is_some())
            .map(|n| (file, n))
    });
    let mut w = Writer {
        trace,
        text: String::new(),
    };
    let mut expected_aliases = Vec::new();
    let mut seen = HashSet::new();
    for file in 0..2 {
        for line in &headers[file].lines {
            if let Some(a) = &line.alias {
                for (_, range) in &a.names {
                    let name = w.value(file, range.clone())?;
                    if seen.insert(name.clone()) {
                        expected_aliases.push(name);
                    }
                }
            }
        }
    }
    let mut moved = Vec::<(usize, Range<usize>)>::new();
    if normalize_target {
        for (n, line) in headers[0].lines.iter().enumerate() {
            if line
                .property
                .as_ref()
                .is_some_and(|(key, _)| key == "title")
            {
                moved.push((0, line.range.clone()));
            } else if (line.alias.is_none() || retained == Some((0, n)))
                && line.range.start < line.content.start
            {
                moved.push((0, line.range.start..line.content.start));
            }
        }
    }
    let mut promoted = Vec::new();
    let mut keys: HashSet<_> = headers[0]
        .lines
        .iter()
        .filter_map(|l| l.property.as_ref().map(|p| p.0.clone()))
        .collect();
    let mut active_names = HashSet::new();
    if let Some((file, n)) = retained {
        for (_, range) in &headers[file].lines[n].alias.as_ref().unwrap().names {
            let mut text = String::new();
            for id in w.trace.interval(file, range.clone())? {
                text.push_str(
                    w.trace
                        .substitution
                        .get(&id)
                        .map(String::as_str)
                        .unwrap_or_else(|| w.trace.original(id)),
                );
            }
            active_names.insert(text);
        }
    }
    let can_consume = retained
        .is_some_and(|(file, n)| headers[file].lines[n].alias.as_ref().unwrap().key.is_some());
    let mut alias_additions = Vec::new();
    for file in 0..2 {
        for (n, line) in headers[file].lines.iter().enumerate() {
            if let Some(a) = &line.alias {
                if retained == Some((file, n)) {
                    if file == 1 {
                        promoted.push(n);
                        if line.range.start < line.content.start {
                            moved.push((file, line.range.start..line.content.start));
                        }
                    }
                    continue;
                }
                if a.movable {
                    if line.range.start < line.content.start {
                        moved.push((file, line.range.start..line.content.start));
                    }
                    let mut protected_duplicate = false;
                    let mut consumed = HashSet::new();
                    let mut relocated = HashSet::new();
                    for ((_, name), token) in a.names.iter().zip(&a.tokens) {
                        if active_names.insert(w.value(file, name.clone())?) {
                            let ids = w.trace.interval(file, token.clone())?;
                            relocated.extend(ids.iter().copied());
                            alias_additions
                                .push(ids.into_iter().map(Placement::Original).collect());
                        } else if token == name {
                            consumed.extend(w.trace.interval(file, name.clone())?);
                        } else {
                            // Keep its property owner too: a retained tag
                            // member must remain a linkable-value occurrence.
                            protected_duplicate = true;
                        }
                    }
                    for id in w.trace.interval(file, line.content.clone())? {
                        if relocated.contains(&id) {
                            continue;
                        }
                        let piece = &w.trace.pieces[id];
                        if consumed.contains(&id)
                            || (!protected_duplicate
                                && can_consume
                                && matches!(piece.role, Role::AliasKey | Role::AliasDelimiter))
                        {
                            w.trace.consumed.push(id);
                        } else if let Some((_, range)) = moved
                            .last_mut()
                            .filter(|(f, r)| *f == file && r.end == piece.range.start)
                        {
                            range.end = piece.range.end;
                        } else {
                            moved.push((file, piece.range.clone()));
                        }
                    }
                } else {
                    return Err(RealizationError::AliasField);
                }
            } else if file == 0 {
                continue;
            } else if line
                .property
                .as_ref()
                .is_some_and(|(key, _)| key != "title" && keys.insert(key.clone()))
            {
                promoted.push(n);
                if line.range.start < line.content.start {
                    moved.push((file, line.range.start..line.content.start));
                }
            } else {
                moved.push((1, line.range.clone()));
            }
        }
    }
    // When the source supplies the retained field, its own members are already
    // present; only later source declarations extend it.
    let header_insert = headers[0]
        .lines
        .iter()
        .rposition(|l| !target_text[l.range.clone()].trim().is_empty())
        .map_or(0, |n| n + 1);
    for (n, line) in headers[0].lines[..header_insert].iter().enumerate() {
        if normalize_target
            && line
                .property
                .as_ref()
                .is_some_and(|(key, _)| key == "title")
        {
            continue;
        }
        if line.alias.is_some() && retained != Some((0, n)) {
            continue;
        }
        if retained == Some((0, n)) {
            w.field(0, line, &alias_additions, !normalize_target)?;
        } else {
            w.original(
                0,
                if normalize_target {
                    line.content.clone()
                } else {
                    line.range.clone()
                },
            )?;
        }
    }
    let has_promoted = !promoted.is_empty() || headers[1].lines.iter().any(|l| l.alias.is_some());
    for n in promoted {
        let line = &headers[1].lines[n];
        w.newline();
        if headers[0].first_root && !normalize_target {
            w.insert(if org { "" } else { "  " });
        }
        if retained == Some((1, n)) {
            w.field(1, line, &alias_additions, false)?;
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
    w.original(0, headers[0].end..target_text.len())?;
    w.newline();
    let leftover = !moved.is_empty();
    let leftover_start = w.text.len();
    if leftover {
        // OG collect-page-properties groups every top-level directive, even
        // below a headline (og 6e7afa8, graph-parser/mldoc.cljc). A quote owns
        // the source directives as body text rather than page properties.
        if org {
            w.insert("*\n#+BEGIN_QUOTE\n");
        } else {
            // An empty title is native, persistent evidence that these old
            // header bytes are body text, including on a headerless next save.
            let property_first = moved.first().is_some_and(|(file, range)| {
                !headers[*file].first_root
                    && headers[*file]
                        .lines
                        .iter()
                        .any(|line| line.range.start == range.start && line.property.is_some())
            });
            w.insert(if property_first { "-\n  " } else { "- " });
        }
        for (i, (file, range)) in moved.iter().enumerate() {
            if i > 0 && !org {
                w.insert("  ");
            }
            w.original(*file, range.clone())?;
        }
        w.newline();
        if org {
            w.insert("#+END_QUOTE\n");
        }
    }
    let leftover_end = w.text.len();
    if let Some(prose) = &headers[1].prose {
        w.prose(1, prose.clone(), org)?;
    }
    w.original(1, headers[1].end..source_text.len())?;
    w.trace.check(&w.text, &w.trace.substitution)?;
    let fmt = if org { Format::Org } else { Format::Md };
    if org && leftover {
        let ast =
            tine_core::render::parse_text_bounded(&w.text[leftover_start..leftover_end], true)
                .ok_or(RealizationError::DirectiveEscape)?;
        if ast
            .blocks
            .iter()
            .any(|block| matches!(block, tine_core::lsdoc::ast::Block::Directive { .. }))
        {
            return Err(RealizationError::DirectiveEscape);
        }
    }
    let read = if org {
        tine_core::org::parse_org(&w.text)
    } else {
        tine_core::doc::parse(&w.text)
    };
    if read
        .pre_block
        .as_deref()
        .and_then(|pre| tine_core::model::page_title_from_preamble(pre, fmt))
        .is_some_and(|name| !refs::same_page(&name, &target.name))
    {
        return Err(RealizationError::SurvivorIdentity);
    }
    let header = tine_core::page_header::page_header_text(
        read.pre_block.as_deref(),
        read.roots.first().map(|b| b.raw()),
    );
    let fields: Vec<_> = header
        .into_iter()
        .flat_map(|h| tine_core::page_header::page_property_lines(h, org))
        .filter(|(k, _)| tine_core::page_header::is_alias_key(k))
        .collect();
    let actual_aliases: Vec<_> = fields
        .iter()
        .flat_map(|(_, v)| tine_core::property_refs::linkable_names(v, org))
        .collect();
    if actual_aliases != expected_aliases || (!expected_aliases.is_empty() && fields.len() != 1) {
        return Err(RealizationError::AliasField);
    }
    let source_body: Vec<&BlockDto> = if headers[1].first_root {
        source.blocks[0]
            .children
            .iter()
            .chain(source.blocks[1..].iter())
            .collect()
    } else {
        source.blocks.iter().collect()
    };
    // Compare the two reader representations directly, carrying the block
    // count into origins. No body text/tree clones or materialized paths.
    fn same_shape<'a>(
        expected: impl IntoIterator<Item = &'a BlockDto>,
        actual: &[tine_core::DocBlock],
    ) -> Option<usize> {
        let mut expected = expected.into_iter();
        let mut count = 0;
        for block in actual {
            let original = expected.next()?;
            count += 1 + same_shape(&original.children, &block.children)?;
        }
        expected.next().is_none().then_some(count)
    }
    let skip = usize::from(headers[0].first_root && !normalize_target);
    let roots = &read.roots;
    let target_prose = usize::from(headers[0].prose.is_some());
    let source_prose = usize::from(headers[1].prose.is_some());
    let target_roots = target.blocks.len() - usize::from(normalize_target) + target_prose;
    let source_start = target_roots + usize::from(leftover);
    let count = source_start + source_prose + source_body.len();
    if roots.len() != count || (leftover && !roots[target_roots].children.is_empty()) {
        return Err(RealizationError::BodyLaw);
    }
    let target_body_count = same_shape(
        &target.blocks[usize::from(normalize_target)..],
        &roots[target_prose..target_roots],
    )
    .ok_or(RealizationError::BodyLaw)?
        - skip;
    let source_body_count = same_shape(
        source_body.iter().copied(),
        &roots[source_start + source_prose..],
    )
    .ok_or(RealizationError::BodyLaw)?;
    if org && leftover {
        let mut original = String::new();
        let alias_ranges: HashSet<_> = headers
            .iter()
            .enumerate()
            .flat_map(|(file, h)| {
                h.lines
                    .iter()
                    .filter(|l| l.alias.is_some())
                    .map(move |l| (file, l.range.start, l.range.end))
            })
            .collect();
        for (file, range) in &moved {
            if alias_ranges.contains(&(*file, range.start, range.end)) {
                continue;
            }
            original.push_str(&w.value(*file, range.clone())?);
        }
        let mut required = HashMap::new();
        let count_refs = |block: &tine_core::DocBlock,
                          counts: &mut HashMap<(bool, String), usize>| {
            for r in block.projection().reference_source().explicit {
                if matches!(r.rule, "explicit_task_marker" | "explicit_priority") {
                    continue;
                }
                *counts
                    .entry((r.rule == "explicit_property_key", refs::page_key(&r.name)))
                    .or_default() += 1;
            }
        };
        for (_, block) in tine_core::page_properties::pre_block_projections(&original, true) {
            count_refs(&block, &mut required);
        }
        let mut actual = HashMap::new();
        count_refs(&roots[target_roots], &mut actual);
        if required
            .iter()
            .any(|(site, n)| actual.get(site).copied().unwrap_or(0) < *n)
        {
            return Err(RealizationError::HiddenOccurrence);
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
        if headers[1].properties && moved.iter().any(|(file, _)| *file == 1) {
            origin.push(Origin::Source(0));
        }
        if moved.iter().any(|(file, _)| *file == 0) {
            if headers[0].properties {
                origin.push(target_property_origin());
            }
        }
        origins.push(origin);
    }
    if source_prose > 0 {
        origins.push(vec![Origin::Source(usize::from(headers[1].properties))]);
    }
    origins.extend((0..source_body_count).map(|j| vec![Origin::Source(source_header + j)]));
    Ok(MergeProposal {
        text: w.text,
        trace: w.trace,
        origins,
        unrealized,
        rewritten,
    })
}

#[cfg(test)]
mod header_property_tests {
    use super::{Format, Header, PageDto};

    fn lines(text: &str, org: bool) -> Vec<Option<(String, String)>> {
        let page = PageDto {
            name: "Header".into(),
            title: "Header".into(),
            kind: tine_core::model::PageKind::Page,
            pre_block: Some(text.into()),
            blocks: Vec::new(),
            rev: None,
            format: if org { Format::Org } else { Format::Md },
            read_only: false,
            guide: false,
        };
        Header::read(&page, text, org)
            .lines
            .into_iter()
            .map(|line| line.property)
            .collect()
    }

    fn property(text: &str, org: bool) -> Option<(String, String)> {
        lines(text, org).into_iter().next().flatten()
    }

    #[test]
    fn merge_recognizes_unicode_page_headers_in_both_formats() {
        assert_eq!(
            property("klíč:: hodnota", false),
            Some(("klíč".into(), "hodnota".into()))
        );
        assert_eq!(
            property("#+klíč: hodnota", true),
            Some(("klíč".into(), "hodnota".into()))
        );
        assert_eq!(
            property("#+a.b/c: value", true),
            Some(("a.b/c".into(), "value".into()))
        );
    }

    #[test]
    fn a_fenced_example_is_never_a_header_property() {
        let prose = "```\nnote:: keep me\n```\nkey:: v\n";
        assert!(lines(prose, false).is_empty());
        assert_eq!(
            lines(&format!("kept:: yes\n{prose}"), false),
            vec![Some(("kept".into(), "yes".into()))],
            "only the leading page property belongs to the header; the fence and following property stay prose"
        );
    }
}
