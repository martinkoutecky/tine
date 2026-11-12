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
    AddedOccurrence,
    SurvivorIdentity,
    AliasField,
    DirectiveEscape,
    FusedLineEnding,
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

#[cfg(test)]
thread_local! {
    /// Drawer-ownership and delimiter comparisons, for the cost test.
    static DRAWER_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[inline]
fn drawer_step() {
    #[cfg(test)]
    DRAWER_STEPS.with(|n| n.set(n.get() + 1));
}

struct Line {
    range: Range<usize>,
    content: Range<usize>,
    property: Option<(String, String)>,
    /// The Org head drawer (index into `Header::drawers`) holding this line.
    drawer: Option<usize>,
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
                    .map(|p| {
                        Alias::read(
                            text,
                            content.clone(),
                            bytes(&p.key_range),
                            bytes(&p.value_range),
                            org,
                        )
                    });
                Line {
                    property: found.map(|p| (p.key.to_ascii_lowercase(), p.value.clone())),
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
                // The field writer copies or replaces the line's terminator.
                let raw = &text[l.content.clone()];
                cuts.push(l.content.end - tine_core::doc::line_terminator(raw).len());
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
    /// `key` and `value` are the parser's byte ranges of this declaration.
    fn read(
        text: &str,
        range: Range<usize>,
        key: Range<usize>,
        value: Range<usize>,
        org: bool,
    ) -> Self {
        let raw = &text[range.clone()];
        let body = &raw[..raw.len() - tine_core::doc::line_terminator(raw).len()];
        let value_start = value.start - range.start;
        let value = &text[value];
        // The parser decides what the declaration and its key are. The model's
        // narrow syntax (§5.2) decides only whether the bytes from the line
        // start through the key's separator are a consumable native key.
        let key = ["alias:: ", "aliases:: ", "#+alias: ", "#+aliases: "]
            .into_iter()
            .find(|k| body[..value_start].starts_with(k) && range.start + k.len() >= key.end)
            .map(|k| range.start..range.start + k.len());
        let mut at = key.as_ref().map_or(value_start, |r| r.end - range.start);
        let mut names = Vec::new();
        let mut tokens = Vec::new();
        let mut syntax = Vec::new();
        let mut plain = key.is_some();
        let mut movable = true;
        // Reader-owned member parsing; never split a bracketed comma again.
        // A plain member's bytes are the parser's (escapes included); the
        // decoded name travels beside them for the reader comparison.
        tine_core::property_refs::for_each_linkable_member(value, org, |name, token, bare| {
            let token = token.map(|r| value_start + r.start..value_start + r.end);
            let found = if bare {
                token
                    .clone()
                    .filter(|r| at <= r.start && r.end <= body.len())
            } else {
                body[at..].find(&name).map(|o| at + o..at + o + name.len())
            };
            let Some(Range { start, end }) = found else {
                plain = false;
                movable = false;
                return;
            };
            let token = token
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
        // The model's consumable alias syntax ends a line with LF or CRLF only
        // (SPEC §5.2); a lone CR stays protected text (RECEIPT-cm-b4-fix3 Q3).
        if matches!(&raw[body.len()..], "\n" | "\r\n") {
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
    /// The terminator of every inserted line (see `propose`).
    eol: &'static str,
    /// A rebuilt active field's terminator: the survivor's first alias
    /// declaration's, else LF (the model's `mergeField`, SPEC §5.1).
    field_end: &'static str,
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
    fn prose(&mut self, file: usize, range: Range<usize>, org: bool) -> Result<(), TraceError> {
        self.insert(if org { "* " } else { "- " });
        let mut at = range.start;
        let lines: Vec<_> = tine_core::doc::physical_lines(&self.trace.inputs[file][range.clone()])
            .map(str::len)
            .collect();
        for len in lines {
            self.original(file, at..at + len)?;
            at += len;
            if at < range.end && !org {
                self.insert("  ");
            }
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
    /// The name the reader takes from a member as written: the parser's
    /// decoded `name`, or the name the rename wrote over its bytes.
    fn decoded(&self, file: usize, name: &str, range: Range<usize>) -> Result<String, TraceError> {
        let ids = self.trace.interval(file, range.clone())?;
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
    fn original(&mut self, file: usize, range: Range<usize>) -> Result<(), TraceError> {
        self.ids(self.trace.interval(file, range)?);
        Ok(())
    }
    fn newline(&mut self) {
        if !self.text.is_empty() && tine_core::doc::line_terminator(&self.text).is_empty() {
            self.insert(self.eol);
        }
    }
    fn field(
        &mut self,
        file: usize,
        line: &Line,
        additions: &[Vec<Placement>],
        rebuild: bool,
        org: bool,
    ) -> Result<(), TraceError> {
        let a = line.alias.as_ref().unwrap();
        let start = line.content.start;
        let raw = &self.trace.inputs[file][line.content.clone()];
        let ending = tine_core::doc::line_terminator(raw).len();
        let end = line.content.end - ending;
        if rebuild {
            self.insert(if org { "#+alias: " } else { "alias:: " });
        } else if a.plain {
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
            if n > 0 || (!rebuild && !a.names.is_empty()) {
                self.insert(", ");
            }
            for p in items {
                match p {
                    Placement::Original(id) => self.ids([*id]),
                    Placement::Insert(s) => self.insert(s),
                }
            }
        }
        if rebuild {
            self.insert(if self.field_end.is_empty() {
                self.eol
            } else {
                self.field_end
            });
        } else {
            self.original(file, end..line.content.end)?;
        }
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
    // The retained declaration is the first survivor alias with members,
    // otherwise the first source one; with no members anywhere, the first
    // declaration. A memberless value (wholly quoted, empty) cannot carry the
    // union. Other fields become protected leftover text unless their entire
    // line is approved native syntax/name tokens.
    let first = |members: bool| {
        (0..2).find_map(|file| {
            headers[file]
                .lines
                .iter()
                .position(|line| {
                    line.alias
                        .as_ref()
                        .is_some_and(|a| !members || !a.names.is_empty())
                })
                .map(|n| (file, n))
        })
    };
    let retained = first(true).or_else(|| first(false));
    let mut w = Writer {
        trace,
        text: String::new(),
        // A new line takes the survivor's convention, as its saves do. The
        // Org model keeps a CRLF's `\r` in its line text, so an Org CRLF
        // survivor gets LF: an inserted empty `*\r\n` headline would not
        // round-trip, leaving it read-only (`valid_merge_keeps_every_line_ending`).
        // A lone CR ends an Org line, so `*\r` round-trips as `*\n`.
        eol: match tine_core::doc::line_convention(target_text) {
            "\r\n" if org => "\n",
            eol => eol,
        },
        after: None,
        fused: false,
        field_end: headers[0]
            .lines
            .iter()
            .find(|line| line.alias.is_some())
            .map_or("\n", |line| {
                tine_core::doc::line_terminator(&target_text[line.content.clone()])
            }),
    };
    let mut expected_aliases = Vec::new();
    let mut seen = HashSet::new();
    let mut rebuild_retained = false;
    for file in 0..2 {
        for (n, line) in headers[file].lines.iter().enumerate() {
            if let Some(a) = &line.alias {
                // Deduplicated by rendered bytes (SPEC §5.1); the reader
                // sees each kept member's decoded name.
                for (decoded, range) in &a.names {
                    let name = w.value(file, range.clone())?;
                    if seen.insert(name) {
                        expected_aliases.push(w.decoded(file, decoded, range.clone())?);
                    } else if retained == Some((file, n)) && !a.plain {
                        rebuild_retained = true;
                    }
                }
            }
        }
    }
    let mut moved = Vec::<(usize, Range<usize>)>::new();
    if normalize_target {
        for line in &headers[0].lines {
            if line
                .property
                .as_ref()
                .is_some_and(|(key, _)| key == "title")
            {
                moved.push((0, line.range.clone()));
            }
        }
    }
    let mut promoted = Vec::new();
    let mut keys: HashSet<_> = headers[0]
        .lines
        .iter()
        .filter_map(|l| l.property.as_ref().map(|p| p.0.clone()))
        .collect();
    // A nonclashing source property joins the target header; a clashing one,
    // a title and unknown text become leftover text (SPEC §5.1).
    let joins: Vec<_> = headers[1]
        .lines
        .iter()
        .map(|line| {
            line.alias.is_none()
                && line
                    .property
                    .as_ref()
                    .is_some_and(|(key, _)| key != "title" && keys.insert(key.clone()))
        })
        .collect();
    // Joining drawer entries go into one head drawer: the target's last, else
    // the first source drawer holding one, placed first. One drawer reads the
    // same in Logseq (mldoc folds a drawer and the directives after it into
    // one; a directive before a drawer hides it) and in Tine for every line
    // ending. Other source drawer delimiters are leftover text.
    let boxed: Vec<_> = (0..headers[1].lines.len())
        .filter(|&n| headers[1].lines[n].drawer.is_some() && (joins[n] || retained == Some((1, n))))
        .collect();
    let target_close = headers[0].drawers.last().map(|d| d.1);
    let kept_drawer = boxed
        .first()
        .filter(|_| target_close.is_none())
        .and_then(|&n| headers[1].lines[n].drawer);
    let mut active_names = HashSet::new();
    if let Some((file, n)) = retained.filter(|_| !rebuild_retained) {
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
                        if !rebuild_retained && line.range.start < line.content.start {
                            moved.push((file, line.range.start..line.content.start));
                        }
                    }
                    if !rebuild_retained {
                        continue;
                    }
                }
                if a.movable {
                    if file == 1 && line.range.start < line.content.start {
                        moved.push((file, line.range.start..line.content.start));
                    }
                    // A rebuilt retained field leaves its protected spacing
                    // and duplicate wrappers with their original property owner.
                    let mut protected_duplicate = retained == Some((file, n));
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
            } else if joins[n] {
                promoted.push(n);
                if line.range.start < line.content.start {
                    moved.push((file, line.range.start..line.content.start));
                }
            } else if !(line.drawer.is_some()
                && line.drawer == kept_drawer
                && headers[1].delimiter(n))
            {
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
    // A target header's structural prefixes are positional: its j-th output
    // line takes its j-th prefix (the first is the block's bullet), whichever
    // line's content lands there. Prefixes left over are protected leftover
    // bytes; a preamble header has none, a normalized one keeps none.
    let kept = |(_, line): &(usize, &Line)| {
        !(normalize_target
            && line
                .property
                .as_ref()
                .is_some_and(|(key, _)| key == "title"))
    };
    let emitted = |(n, line): &(usize, &Line)| line.alias.is_none() || retained == Some((0, *n));
    let slots: Vec<_> = headers[0].lines[..header_insert]
        .iter()
        .enumerate()
        .filter(kept)
        .map(|(_, line)| line.range.start..line.content.start)
        .filter(|r| !r.is_empty())
        .collect();
    let used = if normalize_target {
        0
    } else {
        let lines = headers[0].lines[..header_insert].iter().enumerate();
        slots
            .len()
            .min(lines.filter(kept).filter(emitted).count() + promoted.len())
    };
    moved.extend(slots[used..].iter().map(|r| (0, r.clone())));
    let mut slots = slots[..used].iter();
    // The model's rebuilt field starts with the native `#+alias: ` key, which
    // cannot sit inside a drawer (RECEIPT-cm-b4-fix4 Q5): refuse, unchanged.
    if rebuild_retained && retained.is_some_and(|(f, n)| headers[f].lines[n].drawer.is_some()) {
        return Err(RealizationError::AliasField);
    }
    promoted.retain(|&n| headers[1].lines[n].drawer.is_none());
    let entries = |w: &mut Writer| -> Result<(), TraceError> {
        for &n in &boxed {
            w.newline();
            let line = &headers[1].lines[n];
            if retained == Some((1, n)) {
                w.field(1, line, &alias_additions, rebuild_retained, org)?;
            } else {
                w.original(1, line.content.clone())?;
            }
        }
        Ok(())
    };
    let source_drawer = |w: &mut Writer| -> Result<(), TraceError> {
        if let Some((open, close)) = kept_drawer.map(|d| headers[1].drawers[d]) {
            w.original(1, headers[1].lines[open].content.clone())?;
            entries(w)?;
            w.newline();
            w.original(1, headers[1].lines[close].content.clone())?;
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
        if let Some(prefix) = slots.next() {
            w.original(0, prefix.clone())?;
        }
        if retained == Some((0, n)) {
            w.field(0, line, &alias_additions, rebuild_retained, org)?;
        } else {
            w.original(0, line.content.clone())?;
        }
    }
    if header_insert == 0 {
        source_drawer(&mut w)?;
    }
    let has_promoted = !promoted.is_empty()
        || !boxed.is_empty()
        || headers[1].lines.iter().any(|l| l.alias.is_some());
    for n in promoted {
        let line = &headers[1].lines[n];
        w.newline();
        if let Some(prefix) = slots.next() {
            w.original(0, prefix.clone())?;
        } else if headers[0].first_root && !normalize_target {
            w.insert("  ");
        }
        if retained == Some((1, n)) {
            w.field(1, line, &alias_additions, rebuild_retained, org)?;
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
    let mut lines: Vec<(usize, usize, Vec<Range<usize>>)> = Vec::new();
    if leftover {
        // OG collect-page-properties groups every top-level directive, even
        // below a headline (og 6e7afa8, graph-parser/mldoc.cljc). A quote owns
        // the source directives as body text rather than page properties.
        // The leftover is laid out by original line: each moved line's
        // fragments stay together on one line, ended by their own terminator
        // (else an inserted one). Structural prefixes and whitespace-only
        // fragments never start a line, where they could open a block; in
        // Markdown they go on the title, which then never reads as a property.
        let mut title = Vec::new();
        let mut line_of = HashMap::new();
        for (file, range) in &moved {
            let header = &headers[*file].lines;
            let first = header.partition_point(|l| l.range.end <= range.start);
            for (n, line) in header.iter().enumerate().skip(first) {
                if line.range.start >= range.end {
                    break;
                }
                let prefix = range.start.max(line.range.start)..range.end.min(line.content.start);
                let content = range.start.max(line.content.start)..range.end.min(line.range.end);
                if prefix.start < prefix.end {
                    title.push((*file, prefix));
                }
                if content.start < content.end {
                    let at = *line_of.entry((*file, n)).or_insert_with(|| {
                        lines.push((*file, n, Vec::new()));
                        lines.len() - 1
                    });
                    lines[at].2.push(content);
                }
            }
        }
        // A whitespace-only remainder within one line joins the title; a
        // blank original line that comes first ends it with its own bytes.
        lines.retain(|(file, _, parts)| {
            let inline = !org
                && parts.iter().all(|r| {
                    let t = &w.trace.inputs[*file][r.clone()];
                    t.trim().is_empty() && tine_core::doc::line_terminator(t).is_empty()
                });
            if inline {
                title.extend(parts.iter().map(|r| (*file, r.clone())));
            }
            !inline
        });
        if org {
            let eol = w.eol;
            w.insert(&format!("*{eol}#+BEGIN_QUOTE{eol}"));
        } else {
            let blank = lines
                .first()
                .is_some_and(|(file, _, parts)| {
                    parts
                        .iter()
                        .all(|r| w.trace.inputs[*file][r.clone()].trim().is_empty())
                })
                .then_some(0);
            w.insert(if title.is_empty() && blank.is_none() {
                "-"
            } else {
                "- "
            });
            for (file, range) in &title {
                w.original(*file, range.clone())?;
            }
            match blank.map(|at| lines.remove(at)) {
                Some((file, _, parts)) => {
                    for range in parts {
                        w.original(file, range)?;
                    }
                }
                None => {
                    let eol = w.eol;
                    w.insert(eol);
                }
            }
        }
        for (file, _, parts) in &lines {
            if !org {
                w.insert("  ");
            }
            for range in parts {
                w.original(*file, range.clone())?;
            }
            if tine_core::doc::line_terminator(&w.text).is_empty() {
                let eol = w.eol;
                w.insert(eol);
            }
        }
        if org {
            let eol = w.eol;
            w.insert(&format!("#+END_QUOTE{eol}"));
        }
    }
    let leftover_end = w.text.len();
    if let Some(prose) = &headers[1].prose {
        w.prose(1, prose.clone(), org)?;
    }
    w.original(1, headers[1].end..source_text.len())?;
    w.trace.check(&w.text, &w.trace.substitution)?;
    if w.fused {
        // In scope: a mixed-ending page (external editors, sync). No
        // convention-following terminator can separate the lines here.
        return Err(RealizationError::FusedLineEnding);
    }
    let fmt = if org { Format::Org } else { Format::Md };
    if org && leftover {
        let quote = tine_core::org::lone_cr_to_lf(&w.text[leftover_start..leftover_end]);
        let ast = tine_core::render::parse_text_bounded(&quote, true)
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
    let source_body = &source.blocks[usize::from(headers[1].first_root)..];
    let source_header_children = if headers[1].first_root {
        source.blocks[0].children.as_slice()
    } else {
        &[]
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
    if roots.len() != count || (!leftover && !source_header_children.is_empty()) {
        return Err(RealizationError::BodyLaw);
    }
    let source_child_count = same_shape(
        source_header_children,
        if leftover {
            &roots[target_roots].children
        } else {
            &[]
        },
    )
    .ok_or(RealizationError::BodyLaw)?;
    let target_body_count = same_shape(
        &target.blocks[usize::from(normalize_target)..],
        &roots[target_prose..target_roots],
    )
    .ok_or(RealizationError::BodyLaw)?
        - skip;
    let source_body_count = same_shape(source_body, &roots[source_start + source_prose..])
        .ok_or(RealizationError::BodyLaw)?;
    if org && leftover {
        // The same lines, read where they came from: as header directives.
        // A wholly moved alias line's members are in the field instead.
        // A line from a head drawer is read inside that drawer, as it was.
        let mut original = String::new();
        for (file, n, parts) in &lines {
            let header = &headers[*file];
            let line = &header.lines[*n];
            if header.delimiter(*n)
                || line.alias.is_some()
                    && parts.iter().map(|r| r.len()).sum::<usize>() == line.content.len()
            {
                continue;
            }
            let drawer = line.drawer.map(|d| header.drawers[d]);
            if let Some((open, _)) = drawer {
                original.push_str(&w.value(*file, header.lines[open].content.clone())?);
            }
            for range in parts {
                original.push_str(&w.value(*file, range.clone())?);
            }
            if tine_core::doc::line_terminator(&original).is_empty() {
                original.push('\n');
            }
            if let Some((_, close)) = drawer {
                original.push_str(&w.value(*file, header.lines[close].content.clone())?);
                if tine_core::doc::line_terminator(&original).is_empty() {
                    original.push('\n');
                }
            }
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
        let original = tine_core::org::lone_cr_to_lf(&original);
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
        // Leftover drawer delimiters before a directive make the reader take
        // `#+title` for a tag (RECEIPT-cm-b4-fix4 Q7): refuse a leftover that
        // reads a reference to a name none of its lines refers to.
        let delimiters = lines
            .iter()
            .any(|(file, n, _)| headers[*file].delimiter(*n));
        if delimiters && actual.keys().any(|site| !required.contains_key(site)) {
            return Err(RealizationError::AddedOccurrence);
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
mod drawer_cost_tests {
    use super::{propose, Header, DRAWER_STEPS};

    /// A survivor head of `drawers` three-line Org property drawers.
    fn survivor(drawers: usize) -> String {
        let mut text = String::new();
        for k in 0..drawers {
            text.push_str(&format!(":PROPERTIES:\n:K{k}: v\n:END:\n"));
        }
        text + "* target\n"
    }

    fn merge(target: &str) -> usize {
        // The store reads the given bytes; this crate's sources touch no path.
        let dir = tempfile::tempdir().unwrap();
        let store = tine_store::Store::open(dir.path(), Default::default())
            .unwrap()
            .0;
        let source = ":PROPERTIES:\n:S: w\n:END:\n* source\n";
        let page = |p: &str, bytes: &str| {
            let id = tine_core::model::PageId::from(p);
            store.page_of(&id, bytes.as_bytes()).unwrap().doc
        };
        let (dst, src) = (page("pages/dst.org", target), page("pages/src.org", source));
        DRAWER_STEPS.with(|n| n.set(0));
        let _ = propose(
            &dst,
            target,
            &src,
            source,
            &Default::default(),
            &Default::default(),
            true,
            store.config().file_name_format,
        );
        DRAWER_STEPS.with(|n| n.get())
    }

    /// Lower bound: every header line's drawer is decided once, from a cursor
    /// that only moves forward, so the comparisons are O(lines + drawers);
    /// every delimiter test is O(1).
    #[test]
    fn drawer_classification_is_linear_in_lines_and_drawers() {
        for drawers in [1, 64, 1024, 8192] {
            let target = survivor(drawers);
            let lines = 3 * drawers + 1 + 4;
            let steps = merge(&target);
            eprintln!("{drawers} drawers, {lines} lines: {steps} comparisons");
            assert!(
                steps <= 2 * (lines + drawers),
                "{drawers} drawers, {lines} lines: {steps} comparisons"
            );
        }
    }

    /// Timing: classifying the head costs a small multiple of parsing it once
    /// (the parser's answer it maps), with 8,192 drawers in a 261 KB head.
    #[test]
    fn drawer_classification_stays_within_a_parse_of_the_head() {
        let target = survivor(8192);
        let page = tine_core::model::PageDto {
            name: "dst".into(),
            title: "dst".into(),
            kind: tine_core::model::PageKind::Page,
            pre_block: Some(target[..target.len() - "* target\n".len()].into()),
            blocks: Vec::new(),
            rev: None,
            format: tine_core::model::Format::Org,
            read_only: false,
            guide: false,
        };
        let best = |f: &mut dyn FnMut()| {
            (0..3)
                .map(|_| {
                    let t = std::time::Instant::now();
                    f();
                    t.elapsed()
                })
                .min()
                .unwrap()
        };
        let parse = best(&mut || {
            std::hint::black_box(tine_core::block_regions::parse_document(&target, true));
        });
        let classify = best(&mut || {
            let header = Header::read(&page, &target, true);
            let delimiters = (0..header.lines.len())
                .filter(|&n| header.delimiter(n))
                .count();
            assert_eq!(delimiters, 2 * 8192);
        });
        eprintln!("classification {classify:?} vs one head parse {parse:?}");
        assert!(
            classify <= parse * 6 + std::time::Duration::from_millis(50),
            "classification {classify:?} vs one head parse {parse:?}"
        );
    }
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
