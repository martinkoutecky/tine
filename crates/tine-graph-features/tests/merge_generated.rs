//! Generated two-page merges (D-20261111-GEN). Seeded alias-line and header
//! shapes go through the real `pages::merge_pages`, the guarded host saves and
//! a reopen. The oracle is the model (SPEC §2.1, §5.1–5.4) read through Tine's
//! parser, never the writer's own trace; Native15c admission of the same
//! generator is `native_generated_merges_are_admitted`.
use std::{collections::BTreeMap, fs, sync::Arc};
use tine_graph_features::pages;
use tine_store::Store;

/// xorshift64*: deterministic across platforms and runs.
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())]
    }
    fn chance(&mut self, num: usize, den: usize) -> bool {
        self.below(den) < num
    }
}

/// One generated physical line: structural prefix, content, terminator.
#[derive(Clone, Debug)]
pub struct GenLine {
    pub prefix: String,
    pub content: String,
    pub end: &'static str,
    pub header: bool,
}

#[derive(Clone, Debug)]
pub struct Case {
    pub org: bool,
    pub src: Vec<GenLine>,
    pub dst: Vec<GenLine>,
}

pub fn text(lines: &[GenLine]) -> String {
    lines
        .iter()
        .map(|l| format!("{}{}{}", l.prefix, l.content, l.end))
        .collect()
}

impl Case {
    pub fn src_text(&self) -> String {
        text(&self.src)
    }
    pub fn dst_text(&self) -> String {
        text(&self.dst)
    }
}

const POOL: [&str; 5] = ["a", "A", "b", "c d", "ns/e"];

fn alias_line(rng: &mut Rng, org: bool) -> String {
    let key = if org {
        rng.pick(&["#+alias:", "#+alias:", "#+aliases:", "#+ALIAS:"])
    } else {
        rng.pick(&["alias::", "alias::", "aliases::", "Alias::"])
    };
    let space = rng.pick(&[" ", " ", " ", "  "]);
    match rng.below(10) {
        0 => key.to_owned(),
        1 => format!("{key}{space}{}", rng.pick(&["\"literal\"", "\"a, b\""])),
        _ => {
            let mut value = String::new();
            for n in 0..1 + rng.below(3) {
                if n > 0 {
                    value.push_str(rng.pick(&[", ", ", ", ",", " , ", ",  "]));
                }
                let name = rng.pick(&POOL);
                let encodings: &[usize] = if name.contains(' ') {
                    &[0, 1, 3]
                } else {
                    &[0, 1, 2, 3]
                };
                value.push_str(&match rng.pick(encodings) {
                    0 => name.to_owned(),
                    1 => format!("[[{name}]]"),
                    2 => format!("#{name}"),
                    _ => format!("#[[{name}]]"),
                });
            }
            let trailing = if rng.chance(1, 8) { " " } else { "" };
            format!("{key}{space}{value}{trailing}")
        }
    }
}

fn page(rng: &mut Rng, org: bool, name: &str) -> Vec<GenLine> {
    // Uniform LF, CRLF or lone CR (a valid page, mldoc `eol_chars`), or
    // mixed within the page: LF/CRLF, or all three.
    let ending = rng.below(5);
    let end = |rng: &mut Rng| match ending {
        0 => "\n",
        1 => "\r\n",
        2 => "\r",
        3 => rng.pick(&["\n", "\r\n"]),
        _ => rng.pick(&["\n", "\r\n", "\r"]),
    };
    let mut entries = Vec::new();
    for _ in 0..rng.pick(&[0, 1, 1, 2, 2, 3]) {
        entries.push(alias_line(rng, org));
    }
    for _ in 0..rng.pick(&[0, 0, 1, 2]) {
        let (key, sep) = if org { ("#+", ": ") } else { ("", ":: ") };
        let (k, v) = rng.pick(&[("tags", "t"), ("type", "[[k]]"), ("title", name)]);
        let at = rng.below(entries.len() + 1);
        entries.insert(at, format!("{key}{k}{sep}{v}"));
    }
    // Markdown headers: none, preamble, first root without or with children.
    let kind = if entries.is_empty() {
        0
    } else if org {
        1
    } else {
        1 + rng.below(3)
    };
    let mut lines = Vec::new();
    for (n, content) in entries.into_iter().enumerate() {
        lines.push(GenLine {
            prefix: match (kind, n) {
                (1, _) => "",
                (_, 0) => "- ",
                _ => "  ",
            }
            .into(),
            content,
            end: end(rng),
            header: true,
        });
    }
    // OG's own pages often separate the property preamble with a blank line.
    if kind == 1 && rng.chance(1, 4) {
        lines.push(GenLine {
            prefix: String::new(),
            content: String::new(),
            end: end(rng),
            header: true,
        });
    }
    if kind == 3 {
        for n in 0..1 + rng.below(2) {
            lines.push(GenLine {
                prefix: "  - ".into(),
                content: format!("{name} child {n}"),
                end: end(rng),
                header: false,
            });
        }
    }
    let bullet = if org { "* " } else { "- " };
    for n in 0..1 + rng.below(2) {
        lines.push(GenLine {
            prefix: bullet.into(),
            content: format!("{name} body {n}"),
            end: end(rng),
            header: false,
        });
        if rng.chance(1, 5) {
            lines.push(GenLine {
                prefix: if org { "** " } else { "  - " }.into(),
                content: format!("{name} deep {n}"),
                end: end(rng),
                header: false,
            });
        }
    }
    if rng.chance(1, 8) {
        lines.last_mut().unwrap().end = "";
    }
    if org {
        // A head drawer, from its own stream so the other draws (and so the
        // seeds' earlier cases) stay as they were. It goes before any
        // directive: a directive before it hides it in OG (RECEIPT-cm-b4-fix4).
        let mut d = Rng::new(rng.0 ^ 0xD4A3_E400);
        if d.chance(1, 2) {
            let mut drawer = vec![":PROPERTIES:".to_owned()];
            for _ in 0..d.pick(&[0, 1, 1, 2, 3]) {
                drawer.push(match d.below(5) {
                    0 | 1 => {
                        let line = alias_line(&mut d, true);
                        let (_, rest) = line.split_once(':').unwrap();
                        format!("{}{rest}", d.pick(&[":ALIAS:", ":ALIAS:", ":alias:"]))
                    }
                    2 => ":TYPE: [[k]]".into(),
                    3 => ":TAGS: t".into(),
                    _ => format!(":TITLE: {name}"),
                });
            }
            drawer.push(":END:".into());
            for (n, content) in drawer.into_iter().enumerate() {
                let end = end(&mut d);
                lines.insert(
                    n,
                    GenLine {
                        prefix: String::new(),
                        content,
                        end,
                        header: true,
                    },
                );
            }
        }
    }
    // A CR before an empty LF line is one CRLF on disk, not two lines: give
    // that empty line the CR too, so each generated line is a physical one.
    for n in 1..lines.len() {
        let empty = lines[n].prefix.is_empty() && lines[n].content.is_empty();
        if lines[n - 1].end == "\r" && empty && lines[n].end.starts_with('\n') {
            lines[n].end = "\r";
        }
    }
    lines
}

/// A hand-written Org pair as generated lines: the header runs to the first
/// heading, so the oracle reads it as it reads a generated one.
pub fn org_case(src: &str, dst: &str) -> Case {
    let lines = |text: &str| {
        let mut header = true;
        physical_lines(text)
            .into_iter()
            .map(|l| {
                header &= !l.starts_with('*');
                let end = ending(l);
                line("", &l[..l.len() - end.len()], end, header)
            })
            .collect()
    };
    Case {
        org: true,
        src: lines(src),
        dst: lines(dst),
    }
}

/// `count` cases from `seed`; the same seed always yields the same cases.
pub fn cases(seed: u64, count: usize) -> Vec<Case> {
    let mut rng = Rng::new(seed);
    (0..count)
        .map(|_| {
            let org = rng.chance(1, 3);
            Case {
                org,
                src: page(&mut rng, org, "src"),
                dst: page(&mut rng, org, "dst"),
            }
        })
        .collect()
}

/// One alias declaration as the parser reads its own line.
pub struct Decl {
    pub names: Vec<String>,
    pub end: &'static str,
    pub quoted: Option<String>,
}

/// One page property as the parser reads it: its generated line, key, value,
/// and whether it sits in the Org head drawer.
pub struct Entry {
    pub line: usize,
    pub key: String,
    pub value: String,
    pub drawer: bool,
}

/// Page properties as the parser reads them: a Markdown line on its own, the
/// Org header (drawer entries need their drawer) as one text.
pub fn header_entries(lines: &[GenLine], org: bool) -> Vec<Entry> {
    let header = lines.iter().take_while(|l| l.header);
    if !org {
        return header
            .enumerate()
            .flat_map(|(n, l)| {
                tine_core::page_header::page_property_lines(&l.content, org)
                    .into_iter()
                    .map(move |(key, value)| Entry {
                        line: n,
                        key,
                        value,
                        drawer: false,
                    })
            })
            .collect();
    }
    let mut text = String::new();
    let mut starts = Vec::new();
    for l in header {
        starts.push(text.len());
        text.push_str(&l.content);
        text.push('\n');
    }
    let regions = tine_core::block_regions::parse_document(&text, org);
    let boxes = regions.property_drawer_boundaries(&text);
    regions
        .page_properties()
        .filter_map(|p| {
            Some(Entry {
                line: starts.iter().rposition(|&s| s <= p.line.0)?,
                key: p.key.to_ascii_lowercase(),
                value: p.value.clone(),
                drawer: boxes
                    .iter()
                    .any(|&(open, end)| open < p.line.0 && p.line.0 < end),
            })
        })
        .collect()
}

/// Which generated lines declare an alias.
fn alias_lines(lines: &[GenLine], org: bool) -> Vec<bool> {
    let mut out = vec![false; lines.len()];
    for e in header_entries(lines, org) {
        out[e.line] |= tine_core::page_header::is_alias_key(&e.key);
    }
    out
}

/// A drawer entry that stays leftover text: a source key the survivor header
/// already has (or an earlier source line declares), or an alias declaration
/// other than the retained one keeping a duplicate member (§5.2: it retains
/// its property owner). Inside the leftover quote a drawer line is no
/// property, so the merge refuses `HiddenOccurrence`; the model does not place
/// it yet (Q6 of RECEIPT-cm-b4-fix4).
pub fn drawer_leftover(case: &Case) -> bool {
    let (target, source) = (
        header_entries(&case.dst, case.org),
        header_entries(&case.src, case.org),
    );
    let alias = |e: &Entry| tine_core::page_header::is_alias_key(&e.key);
    let members = |e: &Entry| tine_core::property_refs::linkable_names(&e.value, case.org);
    let all: Vec<_> = target
        .iter()
        .map(|e| (false, e))
        .chain(source.iter().map(|e| (true, e)))
        .collect();
    let retained = all
        .iter()
        .position(|(_, e)| alias(e) && !members(e).is_empty());
    let mut keys: std::collections::HashSet<_> = target.iter().map(|e| e.key.clone()).collect();
    let mut names = std::collections::HashSet::new();
    all.iter().enumerate().any(|(n, &(from_source, e))| {
        if alias(e) {
            let duplicate = members(e)
                .into_iter()
                .fold(false, |d, name| !names.insert(name) || d);
            duplicate && e.drawer && retained != Some(n)
        } else {
            from_source && e.drawer && e.key != "title" && !keys.insert(e.key.clone())
        }
    })
}

/// The Org header's head drawers as the parser finds them: whether each is
/// empty (Tine's reader takes an empty one for prose, mldoc for an empty
/// property list).
pub fn drawers(lines: &[GenLine], org: bool) -> Vec<bool> {
    if !org {
        return Vec::new();
    }
    let header: String = lines
        .iter()
        .take_while(|l| l.header)
        .map(|l| format!("{}\n", l.content))
        .collect();
    tine_core::block_regions::parse_document(&header, org)
        .property_drawer_boundaries(&header)
        .into_iter()
        .map(|(open, end)| header[open..end].matches('\n').count() == 1)
        .collect()
}

/// The source has a head drawer, whose delimiters may become leftover text.
/// In the leftover quote they can make the reader take a following line for a
/// tag, so the merge refuses a leftover reading a reference none of its lines
/// has (`AddedOccurrence`, or `HiddenOccurrence` for a directive it hides).
/// The guard is coarse and may refuse a sound merge: Q7 of RECEIPT-cm-b4-fix4.
pub fn leftover_delimiters(case: &Case) -> bool {
    !drawers(&case.src, case.org).is_empty()
}

/// The retained declaration sits in a drawer: a rebuilt field's `#+alias: `
/// key cannot, so the merge refuses `AliasField` (Q5 of RECEIPT-cm-b4-fix4).
pub fn retained_in_drawer(case: &Case) -> bool {
    let entries = header_entries(&case.dst, case.org)
        .into_iter()
        .chain(header_entries(&case.src, case.org));
    entries
        .filter(|e| tine_core::page_header::is_alias_key(&e.key))
        .find(|e| !tine_core::property_refs::linkable_names(&e.value, case.org).is_empty())
        .is_some_and(|e| e.drawer)
}

/// lsdoc's debug-build span tripwire on a CRLF quote holding an empty drawer
/// and a spaced tag (follow-up, not a merge defect: rendering such a block
/// trips it too).
pub const LSDOC_TRIPWIRE: &str = "OriginCursor queried backwards";

pub fn decls(lines: &[GenLine], org: bool) -> Vec<Decl> {
    header_entries(lines, org)
        .into_iter()
        .filter(|e| tine_core::page_header::is_alias_key(&e.key))
        .map(|Entry { line, value, .. }| {
            let trimmed = value.trim();
            Decl {
                names: tine_core::property_refs::linkable_names(&value, org),
                end: lines[line].end,
                quoted: (trimmed.len() > 1 && trimmed.starts_with('"') && trimmed.ends_with('"'))
                    .then(|| trimmed.to_owned()),
            }
        })
        .collect()
}

/// What the model says the survivor must be.
pub struct Expected {
    /// Survivor-first union, deduplicated by exact rendered bytes (§5.1).
    pub aliases: Vec<String>,
    /// Round 15c's field ending: the survivor's first declaration's, else LF.
    pub end_15c: &'static str,
    /// The retained declaration's own ending (round 15f, pending).
    pub end_own: &'static str,
}
impl Expected {
    /// Q1 of RECEIPT-cm-b4-fix2: 15c and 15f disagree on the field ending.
    pub fn pending_15f(&self) -> bool {
        !self.aliases.is_empty() && self.end_15c != self.end_own
    }
}

/// Declarations exist but none has members: whether they stay in the
/// survivor's header is Q2 of RECEIPT-cm-b4-gen (Native15c rejects every
/// output that keeps one there).
pub fn memberless_only(case: &Case) -> bool {
    let all: Vec<_> = decls(&case.dst, case.org)
        .into_iter()
        .chain(decls(&case.src, case.org))
        .collect();
    !all.is_empty() && all.iter().all(|d| d.names.is_empty())
}

pub fn expected(case: &Case) -> Expected {
    let target = decls(&case.dst, case.org);
    let source = decls(&case.src, case.org);
    let mut aliases = Vec::new();
    for name in target.iter().chain(&source).flat_map(|d| &d.names) {
        if !aliases.contains(name) {
            aliases.push(name.clone());
        }
    }
    Expected {
        aliases,
        end_15c: target.first().map_or("\n", |d| d.end),
        end_own: target
            .iter()
            .chain(&source)
            .find(|d| !d.names.is_empty())
            .map_or("\n", |d| d.end),
    }
}

fn header_aliases(doc: &tine_core::model::PageDto, org: bool) -> Vec<String> {
    tine_core::page_header::page_header_text(
        doc.pre_block.as_deref(),
        doc.blocks.first().map(|b| b.raw.as_str()),
    )
    .into_iter()
    .flat_map(|header| tine_core::page_header::page_property_lines(header, org))
    .filter(|(k, _)| tine_core::page_header::is_alias_key(k))
    .flat_map(|(_, value)| tine_core::property_refs::linkable_names(&value, org))
    .collect()
}

fn first_lines(blocks: &[tine_core::model::BlockDto], out: &mut Vec<String>) {
    for b in blocks {
        out.push(
            b.raw
                .split('\n')
                .next()
                .unwrap_or("")
                .trim_end_matches('\r')
                .to_owned(),
        );
        first_lines(&b.children, out);
    }
}

/// Physical lines with terminators: CRLF, LF or a lone CR ends a line (the
/// oracle's own split, independent of the writer's).
fn physical_lines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let end = rest.find(['\r', '\n']).map_or(rest.len(), |at| {
            at + if rest[at..].starts_with("\r\n") { 2 } else { 1 }
        });
        out.push(&rest[..end]);
        rest = &rest[end..];
    }
    out
}

fn ending(line: &str) -> &'static str {
    ["\r\n", "\n", "\r"]
        .into_iter()
        .find(|e| line.ends_with(e))
        .unwrap_or("")
}

/// The output's page-header lines that declare aliases, with terminators.
fn header_alias_lines<'a>(output: &'a str, org: bool) -> Vec<&'a str> {
    let lines = physical_lines(output);
    if org {
        // The Org header is read as one text: a drawer entry needs its drawer.
        let header: Vec<_> = lines
            .into_iter()
            .take_while(|l| !l.starts_with('*'))
            .collect();
        let generated: Vec<_> = header
            .iter()
            .map(|l| GenLine {
                prefix: String::new(),
                content: l.trim_end_matches(['\r', '\n']).to_owned(),
                end: "",
                header: true,
            })
            .collect();
        return alias_lines(&generated, org)
            .into_iter()
            .zip(header)
            .filter_map(|(alias, line)| alias.then_some(line))
            .collect();
    }
    let first_root = lines.first().is_some_and(|l| l.starts_with("- "));
    let mut found = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        let content = if first_root {
            if n > 0 && !(line.starts_with("  ") && !line.starts_with("  - ")) {
                break;
            }
            &line[2..]
        } else {
            if line.starts_with('-') {
                break;
            }
            *line
        };
        let body = content.trim_end_matches(['\r', '\n']);
        if tine_core::page_header::page_property_lines(body, org)
            .iter()
            .any(|(k, _)| tine_core::page_header::is_alias_key(k))
        {
            found.push(*line);
        }
    }
    found
}

/// Occurrences of `needle`; one ending in a lone CR must not be fused into a
/// CRLF by the byte after it.
fn count(haystack: &str, needle: &str) -> usize {
    haystack
        .match_indices(needle)
        .filter(|(at, _)| {
            !needle.ends_with('\r') || !haystack[at + needle.len()..].starts_with('\n')
        })
        .count()
}

fn hashes(text: &str) -> usize {
    text.match_indices('#')
        .filter(|(at, _)| !text[at + 1..].starts_with('+'))
        .count()
}

/// A failed model obligation: its class and a readable detail.
pub type Failure = (&'static str, String);

/// Checks one actual survivor against the model, independently of the writer.
pub fn check_output(case: &Case, out: &str, doc: &tine_core::model::PageDto) -> Vec<Failure> {
    let org = case.org;
    let want = expected(case);
    let mut failures = Vec::new();
    let mut fail = |class, detail: String| failures.push((class, detail));
    let got = header_aliases(doc, org);
    if got != want.aliases {
        fail(
            "alias-union",
            format!("reader {got:?}, model {:?}", want.aliases),
        );
    }
    let fields = header_alias_lines(out, org);
    if !want.aliases.is_empty() {
        if fields.len() != 1 {
            fail(
                "field-count",
                format!("{} alias header lines", fields.len()),
            );
        } else {
            let end = ending(fields[0]);
            if !want.pending_15f() && end != want.end_15c {
                fail(
                    "field-ending",
                    format!("{:?} ends {end:?}, model {:?}", fields[0], want.end_15c),
                );
            }
        }
    }
    // Every non-alias line survives with its own terminator, multiplicity kept.
    let (dst_alias, src_alias) = (alias_lines(&case.dst, org), alias_lines(&case.src, org));
    let mut needles = BTreeMap::<String, usize>::new();
    let kept = case
        .dst
        .iter()
        .zip(dst_alias)
        .chain(case.src.iter().zip(src_alias));
    for l in kept.filter(|(_, alias)| !alias).map(|(l, _)| l) {
        *needles
            .entry(format!("{}{}", l.content, l.end))
            .or_default() += 1;
    }
    for (needle, n) in needles {
        if count(out, &needle) < n {
            fail("line-lost", format!("{needle:?} x{n}"));
        }
    }
    // Protected syntax is never consumed (§5.2): wrappers, quotes, tag marks.
    let input = format!("{}{}", case.dst_text(), case.src_text());
    for w in ["[[", "]]", "\""] {
        if count(out, w) < count(&input, w) {
            fail("protected-lost", format!("{w:?}"));
        }
    }
    if hashes(out) < hashes(&input) {
        fail("protected-lost", "\"#\"".into());
    }
    for d in decls(&case.dst, org).iter().chain(&decls(&case.src, org)) {
        if let Some(q) = &d.quoted {
            if !out.contains(q.as_str()) {
                fail("protected-lost", format!("{q:?}"));
            }
        }
    }
    // A uniform pair stays uniform (inserted lines follow it); an Org CRLF
    // survivor takes LF for inserted lines (its codec, RECEIPT-cm-b4-fix2).
    let uniform = |lines: &[GenLine]| {
        let ends: Vec<_> = lines
            .iter()
            .map(|l| l.end)
            .filter(|e| !e.is_empty())
            .collect();
        ends.first()
            .copied()
            .filter(|e| ends.iter().all(|x| x == e))
    };
    if let (Some(a), Some(b)) = (uniform(&case.dst), uniform(&case.src)) {
        if a == b && !(org && a == "\r\n") {
            for line in physical_lines(out) {
                let is_field = fields.first() == Some(&line);
                let end = ending(line);
                if !end.is_empty() && end != a && !(is_field && want.pending_15f()) {
                    fail("mixed-endings", format!("{line:?} in a {a:?} pair"));
                }
            }
        }
    }
    let lone_cr = |t: &str| physical_lines(t).into_iter().any(|l| ending(l) == "\r");
    if lone_cr(out) && !lone_cr(&input) {
        fail("bare-cr", "a lone CR no input had".into());
    }
    if out.contains("\r\n") && !input.contains("\r\n") {
        fail("fused-crlf", "a CRLF no input had".into());
    }
    // Bodies stay separate and in order: target's, then source's (§5.1).
    let mut rows = Vec::new();
    first_lines(&doc.blocks, &mut rows);
    let mut at = 0;
    for l in case.dst.iter().chain(&case.src).filter(|l| !l.header) {
        match rows[at..].iter().position(|r| *r == l.content) {
            Some(n) => at += n + 1,
            None => {
                fail(
                    "body-order",
                    format!("{:?} missing or out of order", l.content),
                );
                break;
            }
        }
    }
    failures
}

/// The outcome of one generated case through the real host.
pub enum Run {
    Merged { failures: Vec<Failure> },
    Refused(String),
}

pub fn run(case: &Case) -> Run {
    let (src, dst) = (case.src_text(), case.dst_text());
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    let ext = if case.org { "org" } else { "md" };
    let source = format!("pages/src.{ext}");
    let target = format!("pages/dst.{ext}");
    fs::write(dir.path().join(&source), &src).unwrap();
    fs::write(dir.path().join(&target), &dst).unwrap();
    let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    let host = crate::test_host(&store);
    let dst_id = tine_core::model::PageId::from(target.as_str());
    let mut failures = Vec::new();
    // The oracle reads each input with the parser line by line; the reader's
    // whole-header projection must agree, or the oracle itself is wrong.
    for (path, lines) in [(&source, &case.src), (&target, &case.dst)] {
        let doc = store
            .page(&tine_core::model::PageId::from(path.as_str()))
            .unwrap()
            .doc;
        let own: Vec<_> = decls(lines, case.org)
            .into_iter()
            .flat_map(|d| d.names)
            .collect();
        if header_aliases(&doc, case.org) != own {
            failures.push(("oracle-context", format!("{path}: {own:?}")));
        }
    }
    if let Err(error) = pages::merge_pages(&store, &host, &source, &target) {
        let unchanged = fs::read_to_string(dir.path().join(&source)).ok() == Some(src)
            && fs::read_to_string(dir.path().join(&target)).ok() == Some(dst);
        return Run::Refused(format!(
            "{error}{}",
            if unchanged { "" } else { " (FILES CHANGED)" }
        ));
    }
    if dir.path().join(&source).exists() {
        failures.push(("source-kept", source.clone()));
    }
    let bytes = fs::read_to_string(dir.path().join(&target)).unwrap();
    let loaded = store.page(&dst_id).unwrap();
    failures.extend(check_output(case, &bytes, &loaded.doc));
    for _ in 0..2 {
        let loaded = store.page(&dst_id).unwrap();
        if let Err(e) = crate::test_host_writer::save_on(
            &host,
            &store,
            tine_store::EditKind::SaveBlock,
            &dst_id,
            tine_store::SaveBase::Existing(loaded.rev),
            &loaded.doc,
        ) {
            failures.push(("save-unstable", format!("save refused: {e:?}")));
            break;
        }
        let now = fs::read_to_string(dir.path().join(&target)).unwrap();
        if now != bytes {
            failures.push(("save-unstable", format!("{now:?}")));
            break;
        }
    }
    let first = store.page(&dst_id).unwrap().doc;
    if let Ok(output) = std::env::var("CM_MERGE_ORACLE") {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        (&src, &dst, case.org).hash(&mut hash);
        let row = serde_json::json!({"source": src, "target": dst, "text": bytes,
            "org": case.org, "document": first});
        let name = format!("gen-{}.json", hash.finish());
        fs::write(std::path::Path::new(&output).join(name), row.to_string()).unwrap();
    }
    drop(host);
    drop(store);
    let reopened = Store::open(dir.path(), Default::default()).unwrap().0;
    let doc = reopened.page(&dst_id).unwrap().doc;
    let raws = |d: &tine_core::model::PageDto| {
        fn walk(blocks: &[tine_core::model::BlockDto], out: &mut Vec<String>) {
            for b in blocks {
                out.push(b.raw.clone());
                walk(&b.children, out);
            }
        }
        let mut out = vec![d.pre_block.clone().unwrap_or_default()];
        walk(&d.blocks, &mut out);
        out
    };
    if raws(&doc) != raws(&first) {
        failures.push(("reopen", "reopened document differs".into()));
    }
    Run::Merged { failures }
}

/// Every failing class with its smallest witness, plus the pending count.
pub struct Report {
    pub merged: usize,
    pub pending: usize,
    /// Refused, files unchanged, because a CR would fuse with a following
    /// LF into one CRLF (model question Q4 of RECEIPT-cm-b4-fix3).
    pub fused: usize,
    /// Refused, files unchanged, or tripped, where RECEIPT-cm-b4-fix4 reports
    /// instead of asserting: Q5, Q6 and the lsdoc tripwire, with a witness.
    pub reported: BTreeMap<&'static str, (usize, String)>,
    pub classes: BTreeMap<String, (usize, String)>,
}

pub fn report(cases: &[Case]) -> Report {
    let mut r = Report {
        merged: 0,
        pending: 0,
        fused: 0,
        reported: BTreeMap::new(),
        classes: BTreeMap::new(),
    };
    for case in cases {
        let pending = expected(case).pending_15f();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(case)));
        let failures = match outcome {
            Ok(Run::Merged { failures, .. }) => {
                r.merged += 1;
                failures
            }
            Ok(Run::Refused(e)) if e == "merge realization: FusedLineEnding" => {
                r.fused += 1;
                Vec::new()
            }
            Ok(Run::Refused(e)) => match e.as_str() {
                "merge realization: HiddenOccurrence" if drawer_leftover(case) => {
                    vec![("Q6 drawer leftover", e)]
                }
                "merge realization: HiddenOccurrence" | "merge realization: AddedOccurrence"
                    if leftover_delimiters(case) =>
                {
                    vec![("Q7 leftover drawer delimiters", e)]
                }
                "merge realization: AliasField" if retained_in_drawer(case) => {
                    vec![("Q5 retained drawer alias", e)]
                }
                _ => vec![("refused", e)],
            },
            Err(p) => {
                let message = p
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default();
                let class = if message.contains(LSDOC_TRIPWIRE) {
                    "lsdoc tripwire"
                } else {
                    "panic"
                };
                vec![(class, message)]
            }
        };
        if pending {
            r.pending += 1;
        }
        for (class, detail) in failures {
            let witness = format!(
                "org={} {detail}\n    source={:?}\n    target={:?}",
                case.org,
                case.src_text(),
                case.dst_text()
            );
            // A refusal's class is its reason; other classes name the check.
            let key = if class == "refused" {
                format!("refused: {detail}")
            } else {
                class.to_owned()
            };
            let entry = if class.starts_with('Q') || class == "lsdoc tripwire" {
                r.reported.entry(class).or_insert((0, witness.clone()))
            } else {
                r.classes.entry(key).or_insert((0, witness.clone()))
            };
            entry.0 += 1;
            if witness.len() < entry.1.len() {
                entry.1 = witness;
            }
        }
    }
    r
}

fn assert_clean(r: &Report, total: usize) {
    eprintln!(
        "generated merges: {total} cases, {} merged, {} pending 15f (field ending not asserted), {} fused-ending refusals (Q4, files unchanged)",
        r.merged, r.pending, r.fused
    );
    for (group, (n, witness)) in &r.reported {
        eprintln!("  reported {group}: {n}\n    {witness}");
    }
    let listing: String = r
        .classes
        .iter()
        .map(|(class, (n, witness))| format!("  {class}: {n}\n    {witness}\n"))
        .collect();
    assert!(r.classes.is_empty(), "failing classes:\n{listing}");
}

pub const SEED: u64 = 0x7E57_15C0;

fn line(prefix: &str, content: &str, end: &'static str, header: bool) -> GenLine {
    GenLine {
        prefix: prefix.into(),
        content: content.into(),
        end,
        header,
    }
}

/// The smallest witness found for each failing class (RECEIPT-cm-b4-gen).
pub fn witnesses() -> Vec<Case> {
    vec![
        // A moved source bullet started a leftover line and opened a block.
        Case {
            org: false,
            src: vec![
                line("- ", "alias:: e", "\n", true),
                line("- ", "src body", "\n", false),
            ],
            dst: vec![
                line("", "alias:: #b, b", "\n", true),
                line("- ", "dst body", "\n", false),
            ],
        },
        // The survivor header block lost its bullet with its first line.
        Case {
            org: false,
            src: vec![
                line("- ", "alias:: a", "\n", true),
                line("- ", "src body 0", "\n", false),
            ],
            dst: vec![
                line("- ", "alias::", "\n", true),
                line("  - ", "dst child 0", "\n", false),
                line("- ", "dst body 0", "\n", false),
            ],
        },
        // Remainders of several Org lines were glued into one line that the
        // parser could not read (a panic, not a refusal).
        Case {
            org: true,
            src: vec![
                line("", "#+type: [[k]]", "\n", true),
                line("", "#+alias: c d,  A", "\n", true),
                line("", "#+aliases: b, [[a]],  b", "\n", true),
                line("", "#+ALIAS: [[b]],  #[[c d]]", "\n", true),
                line("* ", "src body 0", "\n", false),
                line("* ", "src body 1", "\n", false),
            ],
            dst: vec![
                line("", "#+alias: #[[c d]], [[c d]], #ns/e", "\r\n", true),
                line("", "#+title: dst", "\r\n", true),
                line("* ", "dst body 0", "\r\n", false),
                line("** ", "dst deep 0", "\r\n", false),
            ],
        },
        // Remainders of several Org lines, glued into one, hid an occurrence
        // the header baseline saw (a HiddenOccurrence refusal).
        Case {
            org: true,
            src: vec![
                line("", "#+ALIAS: #b", "\n", true),
                line("", "#+title: src", "\n", true),
                line("", "#+tags: t", "\n", true),
                line("", "#+alias:  [[A]],[[ns/e]],b ", "\n", true),
                line("* ", "src body 0", "\n", false),
                line("** ", "src deep 0", "\n", false),
            ],
            dst: vec![
                line("", "#+tags: t", "\n", true),
                line("", "#+alias:  b", "\n", true),
                line("", "#+alias: #a ", "\n", true),
                line("", "#+aliases:", "\n", true),
                line("", "", "\r\n", true),
                line("* ", "dst body 0", "\n", false),
            ],
        },
    ]
}

/// The bounded T1 run: the witnesses plus a fixed-seed sample, in seconds.
#[test]
fn generated_merges_follow_the_model() {
    let _guard = crate::integration_lock();
    let mut cases = witnesses();
    cases.extend(self::cases(SEED, 32));
    assert_clean(&report(&cases), cases.len());
}

/// The model refuses a content pair in different formats (§4.2, 4): nothing
/// changes on disk, whatever the alias lines hold.
#[test]
fn generated_mixed_format_pairs_refuse_unchanged() {
    let _guard = crate::integration_lock();
    for case in cases(SEED ^ 1, 4) {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        let src = case.src_text();
        // A survivor of the other format, from its own generated page.
        let dst_text = cases(SEED ^ 2, 16)
            .into_iter()
            .find(|c| c.org != case.org)
            .unwrap()
            .dst_text();
        let (s, t) = if case.org {
            ("pages/src.org", "pages/dst.md")
        } else {
            ("pages/src.md", "pages/dst.org")
        };
        fs::write(dir.path().join(s), &src).unwrap();
        fs::write(dir.path().join(t), &dst_text).unwrap();
        let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
        let host = crate::test_host(&store);
        assert!(pages::merge_pages(&store, &host, s, t).is_err());
        assert_eq!(fs::read_to_string(dir.path().join(s)).unwrap(), src);
        assert_eq!(fs::read_to_string(dir.path().join(t)).unwrap(), dst_text);
    }
}

/// The larger run, made once for the receipt: `CM_GEN_COUNT` cases.
#[test]
#[ignore]
fn generated_merges_follow_the_model_large() {
    let _guard = crate::integration_lock();
    let n = std::env::var("CM_GEN_COUNT").map_or(2000, |v| v.parse().unwrap());
    let cases = cases(SEED ^ 0xBEEF, n);
    assert_clean(&report(&cases), cases.len());
}
