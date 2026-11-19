//! Physical merge realization: the survivor file the model's merge rule O6
//! describes (`cm-cut-model-r6.md`). Extends the reference span writer rather
//! than serializing either body's DTO. O(changed file bytes), no I/O; the
//! caller queues the checked bytes through PagePlan's guarded step.
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};
use tine_core::{
    doc::native::{NativeDocument as Header, NativeLine as Line},
    model::PageDto,
    refs,
    text_trace::{self, Placement, Role, TextTrace, TraceError},
};
mod acceptance;

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

#[cfg(test)]
thread_local! {
    static DRAWER_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static BODY_LINES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static BODY_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
fn members(line: &Line) -> bool {
    line.alias.as_ref().is_some_and(|a| !a.names.is_empty())
}

/// Where the survivor's body ends with one terminator: Org omits all final
/// line breaks from its last block's content; the join keeps one original
/// terminator and the remaining unread breaks are layout (O6/P4).
fn join_end(h: &Header) -> usize {
    h.map.read_end().max(h.end)
}

/// Markdown root blocks indented in the file: the layout prefix of each body
/// line that normalizing a root's subtree to column 0 removes (P4), and the
/// lines visited. The source map gives each block's indentation and dedent.
fn body_prefixes(h: &Header, org: bool) -> (Vec<Range<usize>>, usize) {
    fn size(b: &tine_core::DocBlock) -> usize {
        1 + b.children.iter().map(size).sum::<usize>()
    }
    let (map, mut prefixes, mut lines, mut n) = (&h.map, Vec::new(), 0, 0);
    for root in h.document.roots.iter().filter(|_| !org) {
        let (offset, blocks) = (map.indent(n), n..n + size(root));
        n = blocks.end;
        for (k, i) in blocks.flat_map(|k| map.block(k).map(move |i| (k, i))) {
            let at = map.line(i).start;
            if at >= h.end && offset > 0 {
                lines += 1;
                let first = i == map.block(k).start;
                let layout = if first { map.indent(k) } else { map.removed(i) };
                if offset.min(layout) > 0 {
                    prefixes.push(at..at + offset.min(layout));
                }
            }
        }
    }
    (prefixes, lines)
}

/// The writer's partition of an input: every boundary the description names
/// (header lines, fields, alias members, prose lines, body payloads, removable
/// prefixes), so each placement is whole pieces; a drawer frame line stays
/// whole with its terminator. O(boundaries).
fn cuts(h: &Header, prefixes: &[Range<usize>]) -> Vec<usize> {
    let map = &h.map;
    let mut cuts = vec![h.end, join_end(h)];
    if let Some(p) = &h.prose {
        let lines = (map.line_at(p.start)..map.lines().end).map(|i| map.line(i));
        let header = lines.take_while(|l| l.start < h.end);
        cuts.extend([p.start, p.end]);
        cuts.extend(header.map(|l| l.end.min(h.end)));
    }
    for l in &h.lines {
        cuts.extend([l.range.start, l.content.start, l.content.end]);
        if !l.frame {
            cuts.push(l.payload.end);
        }
        if let Some(a) = &l.alias {
            cuts.extend([a.value, l.unterminated_end]);
            let members = a.names.iter().map(|(_, r)| r).chain(&a.tokens);
            cuts.extend(members.flat_map(|r| [r.start, r.end]));
        }
    }
    let blocks = (0..).take_while(|&n| map.block_start(n).is_some());
    let lines = blocks.flat_map(|n| map.block(n));
    cuts.extend(lines.flat_map(|i| [map.payload(i).start, map.end(i)]));
    cuts.extend(prefixes.iter().flat_map(|r| [r.start, r.end]));
    cuts
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
    fn payload(
        &self,
        file: usize,
        range: Range<usize>,
        raw: &mut String,
    ) -> Result<(), RealizationFailure> {
        for id in self.interval(file, range)? {
            if self.trace.wrappers.contains(&id) {
                raw.push_str("[[");
            }
            raw.push_str(
                self.trace
                    .substitution
                    .get(&id)
                    .map(String::as_str)
                    .unwrap_or_else(|| self.trace.original(id)),
            );
            if self.trace.wrappers.contains(&id) {
                raw.push_str("]]");
            }
        }
        Ok(())
    }
    fn interval(&self, file: usize, range: Range<usize>) -> Result<Vec<usize>, RealizationFailure> {
        self.trace
            .interval(file, range)
            .map_err(|error| RealizationError::Trace(error).at(file.into()))
    }
    /// Promote exactly the input reader's prose range, with its last line's
    /// terminator. Its excluded suffix is layout; no writer-owned blank-line
    /// rule determines content.
    fn prose(&mut self, file: usize, header: &Header, org: bool) -> Result<(), RealizationFailure> {
        let (p, map) = (header.prose.clone().unwrap(), &header.map);
        self.insert(if org { "* " } else { "- " });
        let lines = (map.line_at(p.start)..map.lines().end).map(|i| map.line(i));
        for (n, line) in lines.take_while(|l| l.start < p.end).enumerate() {
            if n > 0 && !org {
                self.insert("  ");
            }
            self.original(file, line.start.max(p.start)..line.end.min(p.end))?;
        }
        let mut separator = p.end;
        if p.end < header.end {
            let last = map.line_at(p.end);
            separator += map.line(last).end - map.end(last);
        }
        self.original(file, p.end..separator)?;
        // Header lines after the prose (Org) are placed with the header.
        let after = header.lines.iter().find(|l| l.range.start >= p.end);
        self.layout(file, separator..after.map_or(header.end, |l| l.range.start))?;
        self.newline();
        Ok(())
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
            Ok(ids
                .into_iter()
                .map(|id| {
                    self.trace
                        .substitution
                        .get(&id)
                        .map(String::as_str)
                        .unwrap_or_else(|| self.trace.original(id))
                })
                .collect())
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
        kept: &[bool],
    ) -> Result<String, RealizationFailure> {
        let mut intended = String::new();
        let keep = |w: &mut Self, range: Range<usize>, intended: &mut String| {
            w.payload(file, range.clone(), intended)?;
            w.original(file, range)
        };
        let a = line.alias.as_ref().unwrap();
        let end = line.unterminated_end;
        let mut at = line.content.start;
        let mut previous = true;
        for (token, &kept) in a.tokens.iter().zip(kept) {
            if kept && !previous {
                self.insert(", ");
                intended.push_str(", ");
                self.consume(file, at..token.start)?;
                at = token.start;
            }
            if kept {
                keep(self, at..token.end, &mut intended)?;
            } else {
                self.consume(file, at..token.end)?;
            }
            at = token.end;
            previous = kept;
        }
        keep(self, at..end, &mut intended)?;
        for ids in additions {
            self.insert(", ");
            intended.push_str(", ");
            for &id in ids {
                let piece = &self.trace.pieces[id];
                self.payload(piece.file, piece.range.clone(), &mut intended)?;
            }
            self.ids(ids.iter().copied());
        }
        self.original(file, end..line.content.end)?;
        Ok(intended)
    }
}

/// How the reader identifies the page an alias member names: a name that
/// parses as a journal day reads as that day, any other by its page key
/// (the model's `Date13.target`, then `page_key`). O(name bytes).
fn reading(name: &str, format: &refs::RenameFormat<'_>) -> String {
    match format.days.and_then(|days| days.parse_text(name)) {
        Some(day) => format!("\u{0}{}", day.ordinal_key()),
        None => refs::page_key(name),
    }
}

/// Realize a merge from the exact bytes guarded by the caller's revisions.
/// `links` is the span writer's already-keyed substitution.
/// The survivor keeps its header and blocks; the source header is allocated
/// as O6 says, in input order. Failure returns no writable proposal.
/// O(changed bytes), no graph scan/I/O.
pub fn propose<'f>(
    target: &PageDto,
    target_text: &str,
    _source: &PageDto,
    source_text: &str,
    links: &HashMap<String, String>,
    org: bool,
    format: impl Into<refs::RenameFormat<'f>>,
) -> Result<MergeProposal, RealizationFailure> {
    let format = format.into();
    let (target_proposal, target_read) =
        refs::rename_described_proposal(target_text, org, links, format)
            .map_err(|e| RealizationError::Trace(e).at(MergePart::Survivor))?;
    let (source_proposal, source_read) =
        refs::rename_described_proposal(source_text, org, links, format)
            .map_err(|e| RealizationError::Trace(e).at(MergePart::Source))?;
    let mut headers = [target_read, source_read];
    let prefixes = headers.each_ref().map(|h| body_prefixes(h, org));
    #[cfg(test)]
    {
        DRAWER_STEPS
            .with(|n| n.set(n.get() + headers.iter().map(|h| h.drawer_steps).sum::<usize>()));
        BODY_LINES.with(|n| n.set(n.get() + prefixes[1].1));
    }
    let prefixes = prefixes.map(|p| p.0);
    for (file, header) in headers.iter().enumerate() {
        if let Some(line) = header
            .lines
            .iter()
            .find(|line| line.alias.as_ref().is_some_and(|a| !a.movable))
        {
            // The reader decodes some legal spellings without native inner-name
            // coordinates. Do not lose or duplicate those alias members.
            return Err(RealizationError::MissingNativeNameRange {
                declaration: [target_text, source_text][file][line.declaration.clone()].into(),
            }
            .at(file.into()));
        }
    }
    // A childless first-root header would promote on the next ordinary save:
    // realize it as a native preamble now (P4: header position is free).
    let normalize_target =
        !org && headers[0].first_root && headers[0].document.roots[0].children.is_empty();
    let target_end = join_end(&headers[0]);
    let mut trace = TextTrace::default();
    let mut unrealized = Vec::new();
    let mut rewritten = HashSet::new();
    let mut protected_alias = HashSet::new();
    for (file, (text, proposal)) in [target_text, source_text]
        .into_iter()
        .zip([target_proposal, source_proposal])
        .enumerate()
    {
        let input = || -> Result<(), RealizationError> {
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
                for prefix in &prefixes[1] {
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
            trace.add_input(text, &edits, &cuts(&headers[file], &prefixes[file]))?;
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
                // A member's whole native spelling belongs to that member:
                // a file link includes its label, path and wrappers. Only the
                // syntax between reader-owned tokens is a delimiter.
                let mut names = a.tokens.iter().peekable();
                let protected =
                    text_trace::alias_nonsyntax(text, line.content.clone(), a.value, &a.tokens);
                let mut protected = protected.iter().peekable();
                for id in trace.interval(file, line.content.clone())? {
                    let r = trace.pieces[id].range.clone();
                    while protected.peek().is_some_and(|p| p.end <= r.start) {
                        protected.next();
                    }
                    if protected.peek().is_some_and(|p| p.start < r.end) {
                        protected_alias.insert(id);
                    }
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
        eol: headers[0].eol,
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
    let mut kept = HashMap::<(usize, usize), Vec<bool>>::new();
    for file in 0..2 {
        for (n, line) in headers[file].lines.iter_mut().enumerate() {
            let Some(a) = &mut line.alias else { continue };
            let kept_members = kept.entry((file, n)).or_default();
            for ((decoded, range), token) in a.names.iter().zip(&a.tokens) {
                let name = w.decoded(file, decoded, range.clone())?;
                let read = reading(&name, &format);
                for id in w.interval(file, token.clone())? {
                    w.trace.readings.insert(id, read.clone());
                }
                let kept = seen.insert(read);
                if kept && retained != Some((file, n)) {
                    additions.push(w.interval(file, token.clone())?);
                }
                kept_members.push(kept);
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
    if let Some(&id) = w
        .trace
        .consumed
        .iter()
        .find(|id| protected_alias.contains(id))
    {
        let piece = &w.trace.pieces[id];
        let line = headers[piece.file]
            .lines
            .iter()
            .find(|line| {
                line.content.start <= piece.range.start && piece.range.end <= line.content.end
            })
            .expect("consumed alias syntax belongs to its header line");
        return Err(RealizationError::MissingNativeNameRange {
            declaration: w.trace.inputs[piece.file][line.declaration.clone()].into(),
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
        let frame = line.blank || line.frame;
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
        headers[1].document.roots[0].children.as_slice()
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
    // Joining fields go after the survivor's last nonblank header line; its
    // trailing blank lines stay where they are (P4).
    let survivor = &headers[0].lines;
    let header_insert = survivor.iter().rposition(|l| !l.blank).map_or(0, |n| n + 1);
    // Each survivor line keeps its own reader-derived prefix. Normalizing a
    // childless header consumes those prefixes; layout never takes a slot.
    let emitted = |n: usize, line: &Line| !members(line) || retained == Some((0, n));
    let first_payload = headers[0]
        .lines
        .iter()
        .position(|l| !l.payload.is_empty())
        .unwrap_or(0);
    // A removed declaration can donate its own reader-derived continuation
    // prefix to a joining field. Retained lines never exchange prefixes.
    let mut spare_prefixes = headers[0].lines[..header_insert]
        .iter()
        .enumerate()
        .filter(|(n, l)| !emitted(*n, l))
        .map(|(_, l)| l.range.start..l.content.start)
        .filter(|r| !r.is_empty());
    let mut expected = acceptance::Intended::default();
    let entries =
        |w: &mut Writer, expected: &mut acceptance::Intended| -> Result<(), RealizationFailure> {
            for &n in &boxed {
                w.newline();
                expected.emit(
                    w,
                    1,
                    &headers[1],
                    n,
                    retained == Some((1, n)),
                    &additions,
                    kept.get(&(1, n)).map_or(&[], Vec::as_slice),
                )?;
            }
            Ok(())
        };
    let source_drawer =
        |w: &mut Writer, expected: &mut acceptance::Intended| -> Result<(), RealizationFailure> {
            if new_drawer {
                w.insert(&format!(":PROPERTIES:{}", w.eol));
                entries(w, expected)?;
                w.newline();
                w.insert(&format!(":END:{}", w.eol));
                w.newline();
            }
            Ok(())
        };
    for (n, line) in headers[0].lines[..header_insert].iter().enumerate() {
        if n == 0 {
            source_drawer(&mut w, &mut expected)?;
        }
        if target_close == Some(n) {
            entries(&mut w, &mut expected)?;
            w.newline();
        }
        if !emitted(n, line) {
            continue;
        }
        if normalize_target && n < first_payload {
            w.layout(0, line.range.clone())?;
            continue;
        }
        let prefix = line.range.start..line.content.start;
        if normalize_target {
            w.layout(0, prefix)?;
        } else {
            w.original(0, prefix)?;
        }
        expected.emit(
            &mut w,
            0,
            &headers[0],
            n,
            retained == Some((0, n)),
            &additions,
            kept.get(&(0, n)).map_or(&[], Vec::as_slice),
        )?;
    }
    if header_insert == 0 {
        source_drawer(&mut w, &mut expected)?;
    }
    for &n in &promoted {
        w.newline();
        if !normalize_target {
            if let Some(prefix) = spare_prefixes.next() {
                w.original(0, prefix)?;
            } else if headers[0].first_root {
                w.insert("  ");
            }
        }
        expected.emit(
            &mut w,
            1,
            &headers[1],
            n,
            retained == Some((1, n)),
            &additions,
            kept.get(&(1, n)).map_or(&[], Vec::as_slice),
        )?;
    }
    for prefix in spare_prefixes {
        w.layout(0, prefix)?;
    }
    w.newline();
    for line in &headers[0].lines[header_insert..] {
        w.original(0, line.range.clone())?;
    }
    if headers[0].prose.is_some() {
        w.prose(0, &headers[0], org)?;
    }
    w.original(0, headers[0].end..target_end)?;
    w.newline();
    // O6's leftover block, top level after the survivor's blocks: an empty
    // first line (Org: a headline with an empty title), then the leftover
    // lines, in Org inside a quote, which owns them as body text (OG groups
    // every top-level directive, even below a headline: og 6e7afa8,
    // graph-parser/mldoc.cljc).
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
    if headers[1].prose.is_some() {
        w.prose(1, &headers[1], org)?;
    }
    let mut at = headers[1].end;
    for prefix in &prefixes[1] {
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
    let counts = expected.check(
        &w,
        &headers,
        &leftover_lines,
        !prefixes[1].is_empty(),
        normalize_target,
        org,
        target,
        &format,
    )?;
    let [target_body_count, source_child_count, source_body_count] = counts.blocks;
    let skip = usize::from(headers[0].first_root && !normalize_target);
    let target_prose = usize::from(headers[0].prose.is_some());
    let source_prose = usize::from(headers[1].prose.is_some());
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
    for _ in 0..usize::from(counts.preamble) + skip {
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

#[cfg(test)]
#[path = "merge_reader_tests.rs"]
mod reader_tests;
