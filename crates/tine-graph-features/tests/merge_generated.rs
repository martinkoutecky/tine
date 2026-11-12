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
const PUNCTUATION: &[u8] = b"!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~";

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
    // Escaped plain members (REG-CM-MERGE-ESCAPED-ALIAS), from their own
    // stream: a backslash before any ASCII punctuation, which Markdown
    // decodes and Org keeps, and one unescaped twin of an escaped spelling.
    // Appending also draws mixed quoted members. The native parser decides
    // whether the original quoted token is now a member or a whole literal.
    let mut e = Rng::new(rng.0 ^ 0xE5CA_9E00);
    for l in lines.iter_mut().filter(|l| l.header) {
        let lower = l.content.to_ascii_lowercase();
        let alias = ["alias::", "aliases::", "#+alias:", "#+aliases:", ":alias:"]
            .iter()
            .any(|k| lower.starts_with(k));
        if alias && e.chance(1, 3) {
            let c = char::from(e.pick(PUNCTUATION));
            let member = match e.below(4) {
                0 => "C\\#".to_owned(),
                1 => "C#".to_owned(),
                2 => format!("p\\{c}q"),
                _ => format!("\\{c}x"),
            };
            let bare = l.content.trim_end().ends_with(':');
            l.content.push_str(if bare { " " } else { ", " });
            l.content.push_str(&member);
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
    pub name_quotes: usize,
}

/// One page property as the parser reads it: generated line, key and value.
pub struct Entry {
    pub line: usize,
    pub key: String,
    pub value: String,
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
    regions
        .page_properties()
        .filter_map(|p| {
            Some(Entry {
                line: starts.iter().rposition(|&s| s <= p.line.0)?,
                key: p.key.to_ascii_lowercase(),
                value: p.value.clone(),
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
            let mut name_quotes = 0;
            tine_core::property_refs::for_each_linkable_member(&value, org, |_, range, plain| {
                if let Some(range) = range.filter(|_| plain) {
                    name_quotes += value[range].matches('"').count();
                }
            });
            Decl {
                names: tine_core::property_refs::linkable_names(&value, org),
                end: lines[line].end,
                quoted: (trimmed.len() > 1 && trimmed.starts_with('"') && trimmed.ends_with('"'))
                    .then(|| trimmed.to_owned()),
                name_quotes,
            }
        })
        .collect()
}

/// What the model says the survivor must be.
pub struct Expected {
    /// Survivor-first union, deduplicated by exact native decoded names.
    pub aliases: Vec<String>,
    /// The selected declaration's own ending.
    pub end_own: &'static str,
}
pub fn expected(case: &Case) -> Expected {
    let target = decls(&case.dst, case.org);
    let source = decls(&case.src, case.org);
    let mut aliases = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for d in target.iter().chain(&source) {
        for name in &d.names {
            if seen.insert(name) {
                aliases.push(name.clone());
            }
        }
    }
    Expected {
        aliases,
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

/// Tag marks: a `#` opening a token. A `#` inside a plain member (`C#`, an
/// escaped `\#`) is member text, which §5.1 deduplicates with its member.
fn hashes(text: &str) -> usize {
    text.match_indices('#')
        .filter(|(at, _)| !text[at + 1..].starts_with('+'))
        .filter(|(at, _)| {
            text[..*at]
                .chars()
                .next_back()
                .is_none_or(|c| c.is_whitespace() || c == ',')
        })
        .count()
}

/// A failed model obligation: its class and a readable detail.
pub type Failure = (&'static str, String);

/// Checks one actual survivor against the model, independently of the writer.
pub fn check_output(
    case: &Case,
    out: &str,
    doc: &tine_core::model::PageDto,
    trace: &tine_core::text_trace::TextTrace,
) -> Vec<Failure> {
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
            let eof_completion =
                want.end_own.is_empty() && end == tine_core::doc::line_convention(&case.dst_text());
            if end != want.end_own && !eof_completion {
                fail(
                    "field-ending",
                    format!("{:?} ends {end:?}, model {:?}", fields[0], want.end_own),
                );
            }
        }
    }
    // Every non-alias line survives with its own terminator, multiplicity kept.
    let (dst_alias, src_alias) = (alias_lines(&case.dst, org), alias_lines(&case.src, org));
    let mut needles = BTreeMap::<String, usize>::new();
    // The old retained-line oracle now accounts separately for the approved
    // native source-boundary exception. Neither a target marker nor an entry
    // is exempted, and the complete trace still checks every original byte.
    let mut approved = BTreeMap::<String, usize>::new();
    if org {
        let source = case.src_text();
        let head_end: usize = case
            .src
            .iter()
            .take_while(|l| l.header)
            .map(|l| l.prefix.len() + l.content.len() + l.end.len())
            .sum();
        let view = tine_core::org::lone_cr_to_lf(&source[..head_end]);
        let regions = tine_core::block_regions::parse_document(&view, true);
        let mut at = 0;
        let physical: BTreeMap<_, _> = tine_core::doc::physical_lines(&source[..head_end])
            .map(|line| {
                let start = at;
                at += line.len();
                (start, (at, line))
            })
            .collect();
        let consumed: std::collections::HashSet<_> = trace.consumed.iter().copied().collect();
        for (open, close) in regions.property_drawer_boundaries(&view) {
            for start in [open, close] {
                let (end, line) = physical[&start];
                let ids = trace.interval(1, start..end).unwrap();
                if ids.len() == 1
                    && consumed.contains(&ids[0])
                    && trace.pieces[ids[0]].role == tine_core::text_trace::Role::DrawerBoundary
                {
                    *approved.entry(line.to_owned()).or_default() += 1;
                }
            }
        }
    }
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
        if count(out, &needle) + approved.get(&needle).copied().unwrap_or(0) < n {
            fail("line-lost", format!("{needle:?} x{n}"));
        }
    }
    // Protected syntax is never consumed (§5.2): wrappers, quotes, tag marks.
    let input = format!("{}{}", case.dst_text(), case.src_text());
    for w in ["[[", "]]"] {
        if count(out, w) < count(&input, w) {
            fail("protected-lost", format!("{w:?}"));
        }
    }
    let name_quotes: usize = decls(&case.dst, org)
        .iter()
        .chain(&decls(&case.src, org))
        .map(|d| d.name_quotes)
        .sum();
    if count(out, "\"") < count(&input, "\"") - name_quotes {
        fail(
            "protected-lost",
            "quotes outside native plain member names".into(),
        );
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
    // A uniform pair stays uniform in both formats.
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
        if a == b {
            for line in physical_lines(out) {
                let end = ending(line);
                if !end.is_empty() && end != a {
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
    let mut inputs = Vec::new();
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
        inputs.push(doc);
    }
    let proof = pages::merge_payload::propose(
        &inputs[1],
        &dst,
        &inputs[0],
        &src,
        &Default::default(),
        &Default::default(),
        case.org,
        store.config().file_name_format,
    );
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
    let proof = proof.expect("a successful operation has a complete text certificate");
    assert_eq!(proof.text, bytes);
    proof
        .trace
        .check(&bytes, &proof.trace.substitution)
        .unwrap();
    let loaded = store.page(&dst_id).unwrap();
    failures.extend(check_output(case, &bytes, &loaded.doc, &proof.trace));
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

/// Every failing class with its smallest witness.
pub struct Report {
    pub merged: usize,
    /// Refused, files unchanged, because a CR would fuse with a following
    /// LF into one CRLF (model question Q4 of RECEIPT-cm-b4-fix3).
    pub fused: usize,
    /// Native extraction tripwires with their witnesses.
    pub reported: BTreeMap<&'static str, (usize, String)>,
    pub classes: BTreeMap<String, (usize, String)>,
}

pub fn report(cases: &[Case]) -> Report {
    let mut r = Report {
        merged: 0,
        fused: 0,
        reported: BTreeMap::new(),
        classes: BTreeMap::new(),
    };
    for case in cases {
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
            Ok(Run::Refused(e)) => vec![("refused", e)],
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
        "generated merges: {total} cases, {} merged, {} fused-ending refusals (Q4, files unchanged)",
        r.merged, r.fused
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
