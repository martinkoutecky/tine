//! Conceptual model conformance, step 2 batch 6 phase A (census Q6 item 1,
//! harness gap H7): the rename's text scanner against the one answer for
//! "what does this block reference".
//!
//! The answerer is `tine_core::reference_evidence` (each block's
//! `projection().reference_source().explicit`, every occurrence with its rule
//! and source range). The second computer is the rename's rewrite,
//! `tine_core::refs::rename_rewrite` (`rename_refs_multi`,
//! `rename_tags_property_multi`, `export_definition::rename_selectors`), which
//! scans the whole file text itself to decide which occurrences a rename
//! changes. The model receives Tine's occurrences as an input, so the two were
//! never compared before this test.
//!
//! How the scanner's occurrences are read without changing it: every
//! candidate name of a page (the evidence names plus a deliberately crude
//! harvest of anything bracketed, hashed or listed in a property value) is
//! renamed in ONE `rename_rewrite` call to a distinct sentinel (`QQR<i>QQ`,
//! no chaining, one pass), and each sentinel in the output is one occurrence
//! the rename rewrites. Both sides' occurrences are then labelled with the same
//! syntactic classifier (`form`), so a difference reads as a form, e.g.
//! `rename-only: [[x]] in {{query}}` or `evidence-only explicit_property_key:
//! property key`. A rename-only occurrence on a page that holds no evidence
//! reference to that name (namespace closure included) is marked `(page not
//! read)`: the planner selects pages by `explicit_referrers`, so today such an
//! occurrence is rewritten only when the same page also references the name
//! somewhere else.
//!
//! Corpora:
//! - the generated forms below (every reference form either side knows, for
//!   several name shapes, in Markdown and Org); differences go to part
//!   `extraction` of `expected-differences.json` (`CM_REFRESH=1` rewrites it),
//!   and the per-text answers go to `frontend-refs/` for
//!   `src/cmConformanceRefs.test.ts`, which runs the frontend's
//!   `swipeRefNames` and `unsavedPathsFor` (through `renameOrMergePage`) on the
//!   same texts;
//! - `CM_ANON=<graph dir>` (ignored test `extraction_anonymized_graph`): every
//!   page of a real graph, reporting forms and counts only, never text or
//!   names, to `artifacts/cm/extraction-anonymized.json`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde_json::{json, Value};
use tine_core::config::FileNameFormat;
use tine_core::doc::DocBlock;
use tine_core::model::{decode_page_name, encode_page_name};
use tine_core::refs::{namespace_refs, page_key, rename_rewrite};

use super::shared;
use super::shared::Section;

/// The graph's file-name format for the corpus (Org file links encode stems
/// with it, and the rename decodes them with it).
const NAME_FORMAT: FileNameFormat = FileNameFormat::TripleLowbar;

/// Name shapes: plain, multi-word, namespaced, non-ASCII, and a task marker /
/// priority letter (which are references but never rewritten).
const NAMES: &[(&str, &str)] = &[
    ("plain", "Old"),
    ("words", "old page"),
    ("ns", "ns/Old"),
    ("unicode", "Über"),
];

/// Every reference form either side knows, plus look-alikes that are not
/// references. `{N}` is the name, `{S}` its file stem under [`NAME_FORMAT`].
/// A `md:` / `org:` prefix limits the form to one format; `pre:` puts the
/// text in the page's pre-block (page properties) instead of a bullet.
const FORMS: &[(&str, &str)] = &[
    ("link", "see [[{N}]] here"),
    ("link-spaced", "see [[ {N} ]] here"),
    ("link-case", "see [[{n}]] here"),
    ("tag", "a #{N} tag"),
    ("tag-bracket", "a #[[{N}]] tag"),
    ("tag-trailing-dot", "end #{N}."),
    ("tag-in-word", "word#{N} here"),
    ("tag-url-fragment", "visit https://ex.com/docs#{N} now"),
    ("heading-tag", "# heading #{N}"),
    ("emphasis", "**[[{N}]]** and _#{N}_"),
    ("embed", "{{embed [[{N}]]}}"),
    ("embed-spaced", "{{embed  [[{N}]] }}"),
    // A block embed names no page (the anonymized graph: evidence answers a
    // page named after the `((uuid))`).
    (
        "embed-block",
        "{{embed ((6571a2b3-1111-2222-3333-444455556666))}} [[{N}]]",
    ),
    ("query-link", "{{query [[{N}]]}}"),
    ("query-and", "{{query (and [[{N}]] (task TODO))}}"),
    ("query-page", "{{query (page \"{N}\")}}"),
    ("query-page-bare", "{{query (page {N})}}"),
    ("query-search", "{{query \"{N}\"}}"),
    ("query-plus-link", "{{query [[{N}]]}} and [[{N}]]"),
    ("tine-query", "{{tine-query @page and name = \"{N}\"}}"),
    (
        "tine-query-link",
        "{{tine-query @block and refs = [[{N}]]}}",
    ),
    ("renderer", "{{renderer :x, [[{N}]]}}"),
    ("cloze", "{{cloze [[{N}]]}}"),
    ("nested", "[[a [[{N}]] c]]"),
    ("nested-outer", "[[{N} [[x]]]]"),
    ("ns-child", "see [[{N}/child]]"),
    ("unmatched-opener", "use [[ to link [[{N}]]"),
    // Nested arrays in a URL's query string (the anonymized graph).
    (
        "url-brackets",
        "see https://ex.com/q?[[[{N}],[2]],[[{N}],[3]]]",
    ),
    ("md:label-link", "[label]([[{N}]])"),
    ("md:inline-code", "literal `[[{N}]]` and `#{N}`"),
    ("md:fence", "```\n[[{N}]] #{N}\n```"),
    ("md:comment", "<!-- [[{N}]] -->"),
    ("md:html", "<span>[[{N}]]</span>"),
    ("math", "$$[[{N}]]$$"),
    ("escaped", "\\[[{N}]] and \\#{N}"),
    ("table", "| a | [[{N}]] |\n| - | - |"),
    ("quote", "> quoted [[{N}]]"),
    ("prop-tags-bare", "body\ntags:: {N}, other"),
    ("prop-tags-link", "body\ntags:: [[{N}]]"),
    ("prop-tags-hash", "body\ntags:: #{N}"),
    ("prop-tags-quoted", "body\ntags:: \"{N}\""),
    ("prop-alias-bare", "body\nalias:: {N}"),
    ("prop-other-link", "body\nrelated:: [[{N}]]"),
    ("prop-other-bare", "body\nrelated:: {N}"),
    ("prop-key", "body\n{N}:: value"),
    ("prop-title-link", "body\ntitle:: [[{N}]]"),
    ("pre:page-tags", "tags:: {N}, other\nalias:: {N} alias"),
    ("pre:page-link", "related:: [[{N}]]"),
    ("pre:page-key", "{N}:: value"),
    ("logbook", "body\n:LOGBOOK:\nCLOCK: [[{N}]]\n:END:"),
    ("org:file-link-label", "[[file:./pages/{S}.org][{N}]]"),
    (
        "org:file-link-other-label",
        "[[file:./pages/{S}.org][a label]]",
    ),
    ("org:file-link-bare", "[[file:./{S}.org]]"),
    ("org:labelled-link", "[[{N}][a label]]"),
    ("org:headline-tags", "heading :{N}:other:"),
    ("org:src", "x\n#+BEGIN_SRC\n[[{N}]] #{N}\n#+END_SRC"),
    ("org:verbatim", "=[[{N}]]= and ~#{N}~"),
    ("org:drawer-property", "x\n:PROPERTIES:\n:tags: {N}\n:END:"),
];

/// Task markers and priorities: references (OG `with-page-refs`), never
/// rewritten (model §5.1, permanent exception).
const MARKER_FORMS: &[(&str, &str, &str)] = &[
    ("marker", "TODO", "TODO write it"),
    ("priority", "A", "TODO [#A] write it"),
];

/// One generated text.
struct Entry {
    id: String,
    name: String,
    is_org: bool,
    text: String,
}

fn corpus() -> Vec<Entry> {
    let mut out = Vec::new();
    for is_org in [false, true] {
        let fmt = if is_org { "org" } else { "md" };
        for (form, template) in FORMS {
            let (only, form) = match form.split_once(':') {
                Some((p, f)) => (Some(p), f),
                None => (None, *form),
            };
            if only == Some(if is_org { "md" } else { "org" }) {
                continue;
            }
            for (shape, name) in NAMES {
                let body = template
                    .replace("{N}", name)
                    .replace("{n}", &name.to_lowercase())
                    .replace("{S}", &encode_page_name(name, NAME_FORMAT));
                out.push(Entry {
                    id: format!("{fmt}/{form}/{shape}"),
                    name: (*name).to_owned(),
                    is_org,
                    text: page_text(&body, is_org, only == Some("pre")),
                });
            }
        }
        for (form, name, body) in MARKER_FORMS {
            out.push(Entry {
                id: format!("{fmt}/{form}/plain"),
                name: (*name).to_owned(),
                is_org,
                text: page_text(body, is_org, false),
            });
        }
    }
    out
}

/// A one-block page holding `body` (continuation lines indented), or a page
/// whose pre-block is `body`.
fn page_text(body: &str, is_org: bool, pre: bool) -> String {
    if pre {
        return format!("{body}\n\n{} body\n", if is_org { "*" } else { "-" });
    }
    let mut lines = body.lines();
    let first = lines.next().unwrap_or_default();
    let mut out = format!("{} {first}\n", if is_org { "*" } else { "-" });
    for line in lines {
        if !is_org {
            out.push_str("  ");
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// The form label of an occurrence whose name sits at `at..end` of `text`:
/// its syntax (what precedes and follows the name) and its context (enclosing
/// macro, nesting, property position, literal region). Test-side reporting
/// only, applied identically to both sides; it decides nothing.
fn form(text: &str, at: usize, end: usize) -> String {
    let before = &text[..at];
    let after = &text[end..];
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let line_end = after.find('\n').map_or(text.len(), |i| end + i);
    let line = &text[line_start..line_end];
    let col = at - line_start;
    let last_open = before.rfind("[[");
    let file_link = last_open
        .is_some_and(|o| text[o + 2..].starts_with("file:") && !text[o..at].contains("]]"));
    // `#x` and `#[[x]]` are one form: the rename may respell one as the other.
    let syntax = if before.ends_with("#[[") {
        "#tag"
    } else if before.ends_with("[[") || before.ends_with("[[ ") {
        if after.starts_with("][") {
            "[[x][label]]"
        } else {
            "[[x]]"
        }
    } else if file_link {
        if text[last_open.unwrap()..at].contains("][") {
            "[[file:…][x]]"
        } else {
            "[[file:…x.org]]"
        }
    } else if before.ends_with("[#") {
        "[#x]"
    } else if before.ends_with('#') {
        "#tag"
    } else if before.ends_with(':') && after.starts_with(':') && !line.contains("::") {
        ":x: (headline tag)"
    } else {
        "bare"
    };
    let mut ctx: Vec<String> = Vec::new();
    // Enclosing macro: the last `{{` before the name not closed before it.
    if let Some(open) = before.rfind("{{") {
        if !before[open..].contains("}}") {
            let name: String = text[open + 2..]
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '}')
                .collect();
            ctx.push(format!("in {{{{{name}}}}}"));
        }
    }
    // Nesting: more than one unclosed `[[` before the name on its line.
    let prefix = &line[..col];
    if prefix.matches("[[").count() > prefix.matches("]]").count() + 1 {
        ctx.push("nested".into());
    }
    // Property position.
    let trimmed = line.trim_start();
    let lead = line.len() - trimmed.len();
    let body = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .unwrap_or(trimmed);
    let body_at = lead + (trimmed.len() - body.len());
    if let Some(sep) = body.find("::") {
        let key = &body[..sep];
        if !key.is_empty() && !key.contains(char::is_whitespace) {
            let key_end = body_at + sep;
            if col < key_end {
                ctx.push("property key".into());
            } else {
                ctx.push(format!("value of {}::", key_class(key)));
            }
        }
    } else if let Some(rest) = body.strip_prefix(':') {
        if let Some(sep) = rest.find(": ") {
            if col > body_at + sep + 1 {
                ctx.push(format!("value of :{}: (drawer)", key_class(&rest[..sep])));
            }
        }
    }
    // Literal regions: inline code on the line, a fence or source block open
    // before the name.
    if prefix.matches('`').count() % 2 == 1 {
        ctx.push("in `code`".into());
    }
    let fences = before
        .lines()
        .filter(|l| l.trim_start().trim_start_matches("- ").starts_with("```"))
        .count();
    let src_open = before.to_ascii_uppercase().matches("#+BEGIN_").count()
        > before.to_ascii_uppercase().matches("#+END_").count();
    if fences % 2 == 1 || src_open {
        ctx.push("in code block".into());
    }
    if before.contains(":LOGBOOK:") && !before.contains(":END:") {
        ctx.push("in logbook".into());
    }
    if ctx.is_empty() {
        syntax.to_owned()
    } else {
        format!("{syntax} {}", ctx.join(", "))
    }
}

fn key_class(key: &str) -> &'static str {
    match key.to_ascii_lowercase().as_str() {
        "tags" => "tags",
        "alias" => "alias",
        "aliases" => "aliases",
        "title" => "title",
        _ => "other",
    }
}

/// One occurrence: the page key it names and its form; `rule` is the
/// evidence rule (empty for the rename side).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Occ {
    key: String,
    form: String,
    rule: &'static str,
}

/// The evidence side: every block of the page (the pre-block through the
/// store's page-header projection), its explicit references.
fn evidence(text: &str, is_org: bool) -> (Vec<Occ>, BTreeSet<String>) {
    let rel = if is_org { "pages/p.org" } else { "pages/p.md" };
    let parsed = shared::parse_file(rel, text);
    let mut out = Vec::new();
    let mut names = BTreeSet::new();
    let mut sites = BTreeSet::new();
    let mut block_no = 0usize;
    let mut take = |block: &DocBlock| {
        block_no += 1;
        let raw = block.raw();
        for r in block.projection().reference_source().explicit {
            let span = &raw[r.range.clone()];
            let at = span
                .find(r.name.as_str())
                .map_or(r.range.start, |i| r.range.start + i);
            let end = (at + r.name.len()).min(raw.len());
            let end = if raw.is_char_boundary(end) { end } else { at };
            // One site, several rules (`#[[x]]` is a tag holding a link):
            // the rename sees one site.
            let key = page_key(&r.name);
            if !sites.insert((block_no, at, key.clone())) {
                names.insert(r.name.clone());
                continue;
            }
            out.push(Occ {
                key,
                form: form(raw, at, end),
                rule: r.rule,
            });
            names.insert(r.name.clone());
        }
    };
    if let Some(pre) = &parsed.pre_block {
        let header = tine_core::page_properties::property_projection_with_config(
            pre,
            is_org,
            &Default::default(),
        );
        take(&header);
    }
    fn walk(blocks: &[DocBlock], take: &mut dyn FnMut(&DocBlock)) {
        for b in blocks {
            take(b);
            walk(&b.children, take);
        }
    }
    walk(&parsed.roots, &mut take);
    (out, names)
}

/// Crude candidate names for the rename side: anything the scanner could
/// possibly rewrite. False candidates cost nothing (the scanner decides).
fn harvest(text: &str, out: &mut BTreeSet<String>) {
    let mut push = |s: &str| {
        let s = s.trim();
        if !s.is_empty() && !s.contains('\n') && s.len() < 200 {
            out.insert(s.to_owned());
        }
    };
    for (at, _) in text.match_indices("[[") {
        let body = &text[at + 2..];
        if let Some(end) = body.find("]]") {
            let body = &body[..end];
            push(body);
            if let Some(path) = body.strip_prefix("file:") {
                let path = path.split("][").next().unwrap_or(path);
                let file = path.rsplit('/').next().unwrap_or(path);
                let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
                push(&decode_page_name(stem, NAME_FORMAT));
            }
            if let Some((target, label)) = body.split_once("][") {
                push(target);
                push(label);
            }
        }
    }
    for (at, _) in text.match_indices('#') {
        let token: String = text[at + 1..]
            .chars()
            .take_while(|c| !c.is_whitespace() && !matches!(c, '[' | ']' | '{' | '}' | '`'))
            .collect();
        push(&token);
        push(token.trim_end_matches(|c: char| !c.is_alphanumeric()));
    }
    for line in text.lines() {
        if let Some((key, value)) = line.split_once("::") {
            push(key.trim_start().trim_start_matches(['-', '*', ' ']));
            for v in value.split([',', '，']) {
                push(v);
            }
        }
        for (i, seg) in line.split(':').enumerate() {
            if i > 0 {
                push(seg);
            }
        }
    }
    for head in ["(page ", "(page-ref ", "name = \""] {
        for (at, _) in text.match_indices(head) {
            let rest = text[at + head.len()..].trim_start_matches('"');
            push(rest.split(['"', ')']).next().unwrap_or(""));
        }
    }
}

/// The rename side: every occurrence `rename_rewrite` rewrites, one pass.
fn renamed(text: &str, is_org: bool, candidates: &BTreeSet<String>) -> Vec<Occ> {
    let mut by_key: BTreeMap<String, String> = BTreeMap::new();
    for c in candidates {
        by_key.entry(page_key(c)).or_insert_with(|| c.clone());
    }
    by_key.retain(|k, _| !k.is_empty());
    let keys: Vec<String> = by_key.keys().cloned().collect();
    let pairs: Vec<(String, String)> = keys
        .iter()
        .enumerate()
        .map(|(i, k)| (by_key[k].clone(), format!("QQR{i}QQ")))
        .collect();
    let output = rename_rewrite(text, is_org, &pairs, NAME_FORMAT);
    let mut out = Vec::new();
    for (i, key) in keys.iter().enumerate() {
        let sentinel = format!("QQR{i}QQ");
        for (at, _) in output.match_indices(&sentinel) {
            out.push(Occ {
                key: key.clone(),
                form: form(&output, at, at + sentinel.len()),
                rule: "",
            });
        }
    }
    out
}

/// One page's comparison. `show` names the key in each difference (generated
/// corpus only; never for a real graph).
struct PageResult {
    diffs: Vec<String>,
    agreed: Vec<String>,
    /// Keys the rename rewrites on this page.
    rewrites: BTreeSet<String>,
    /// Keys whose referrer set holds this page (`explicit_referrers` input).
    referrer: BTreeSet<String>,
}

fn compare(text: &str, is_org: bool, show: bool) -> PageResult {
    let (ev, names) = evidence(text, is_org);
    let mut candidates = names.clone();
    harvest(text, &mut candidates);
    let rn = renamed(text, is_org, &candidates);
    let referrer: BTreeSet<String> = names
        .iter()
        .flat_map(|n| namespace_refs(n).map(page_key).collect::<Vec<_>>())
        .collect();
    let rewrites: BTreeSet<String> = rn.iter().map(|o| o.key.clone()).collect();
    let mut left: BTreeMap<(String, String), Vec<&'static str>> = BTreeMap::new();
    for o in &ev {
        left.entry((o.key.clone(), o.form.clone()))
            .or_default()
            .push(o.rule);
    }
    let mut right: BTreeMap<(String, String), usize> = BTreeMap::new();
    for o in &rn {
        *right.entry((o.key.clone(), o.form.clone())).or_default() += 1;
    }
    let mut diffs = Vec::new();
    let mut agreed = Vec::new();
    let all: BTreeSet<(String, String)> = left.keys().chain(right.keys()).cloned().collect();
    for k in all {
        let rules = left.get(&k).cloned().unwrap_or_default();
        let r = right.get(&k).copied().unwrap_or(0);
        let who = if show {
            format!(" [{}]", k.0)
        } else {
            String::new()
        };
        for _ in 0..rules.len().min(r) {
            agreed.push(k.1.clone());
        }
        for rule in rules.iter().skip(r) {
            diffs.push(format!("evidence-only {rule}: {}{who}", k.1));
        }
        for _ in rules.len()..r {
            let read = if referrer.contains(&k.0) {
                ""
            } else {
                " (page not read: no evidence reference)"
            };
            diffs.push(format!("rename-only: {}{read}{who}", k.1));
        }
    }
    diffs.sort();
    PageResult {
        diffs,
        agreed,
        rewrites,
        referrer,
    }
}

/// The generated corpus: differences by entry, and the frontend cases.
#[test]
fn rename_scanner_vs_reference_evidence() {
    let mut section = Section::default();
    let mut cases: BTreeMap<String, Value> = BTreeMap::new();
    for entry in corpus() {
        let result = compare(&entry.text, entry.is_org, true);
        if !result.diffs.is_empty() {
            section.fixtures.insert(entry.id.clone(), result.diffs);
        }
        let rel = if entry.is_org {
            "pages/p.org"
        } else {
            "pages/p.md"
        };
        let parsed = shared::parse_file(rel, &entry.text);
        let mut blocks = Vec::new();
        fn walk(blocks: &[DocBlock], out: &mut Vec<Value>) {
            for b in blocks {
                let keys: BTreeSet<String> = b
                    .projection()
                    .reference_source()
                    .explicit
                    .iter()
                    .map(|r| page_key(&r.name))
                    .collect();
                out.push(json!({"raw": b.raw(), "refs": keys}));
                walk(&b.children, out);
            }
        }
        walk(&parsed.roots, &mut blocks);
        let renames: BTreeMap<String, Value> = result
            .rewrites
            .iter()
            .chain(result.referrer.iter())
            .chain(std::iter::once(&page_key(&entry.name)))
            .map(|k| {
                (
                    k.clone(),
                    json!({
                        "rewrites": result.rewrites.contains(k),
                        "referrer": result.referrer.contains(k),
                    }),
                )
            })
            .collect();
        let case = cases
            .entry(entry.id.split('/').next().unwrap().to_owned())
            .or_insert_with(|| json!({}));
        case[entry.id.clone()] = json!({
            "name": entry.name,
            "preBlock": parsed.pre_block,
            "blocks": blocks,
            "renames": renames,
        });
    }
    super::check_frontend_cases(
        "frontend-refs",
        cases,
        "CM_REFRESH=1 cargo test -p tine-store --lib cm_conformance_tests::extraction, \
         then CM_REFRESH=1 npx vitest run src/cmConformanceRefs.test.ts",
    );
    shared::check_expected(
        env!("CARGO_MANIFEST_DIR"),
        "extraction",
        &section,
        "CM_REFRESH=1 cargo test -p tine-store --lib cm_conformance_tests::extraction",
    );
}

/// A real graph (`CM_ANON=<graph dir>`): forms and counts only. Writes
/// `artifacts/cm/extraction-anonymized.json` in the worktree and prints the
/// same summary; never prints a name or a text.
#[test]
#[ignore = "needs CM_ANON=<graph dir>"]
fn extraction_anonymized_graph() {
    let Some(root) = std::env::var_os("CM_ANON") else {
        return;
    };
    let root = Path::new(&root);
    let mut files = Vec::new();
    for dir in ["pages", "journals"] {
        let Ok(read) = fs::read_dir(root.join(dir)) else {
            continue;
        };
        for e in read.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "md" || x == "org") {
                files.push(p);
            }
        }
    }
    files.sort();
    let mut diffs: BTreeMap<String, usize> = BTreeMap::new();
    let mut agreed: BTreeMap<String, usize> = BTreeMap::new();
    let mut pages_with_diff: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    let mut skipped = 0usize;
    // Rough cost of the two computers per page (debug build: a ratio only):
    // the rename's one-pair scan and the evidence parse of every block.
    let (mut scan_ns, mut evidence_ns) = (0u128, 0u128);
    for (i, p) in files.iter().enumerate() {
        let Ok(text) = fs::read_to_string(p) else {
            skipped += 1;
            continue;
        };
        let is_org = p.extension().is_some_and(|x| x == "org");
        let t = std::time::Instant::now();
        let _ = rename_rewrite(
            &text,
            is_org,
            &[("Zq absent".into(), "x".into())],
            NAME_FORMAT,
        );
        scan_ns += t.elapsed().as_nanos();
        let t = std::time::Instant::now();
        let _ = evidence(&text, is_org);
        evidence_ns += t.elapsed().as_nanos();
        let Ok(result) = std::panic::catch_unwind(|| compare(&text, is_org, false)) else {
            skipped += 1;
            continue;
        };
        for d in result.diffs {
            *diffs.entry(d.clone()).or_default() += 1;
            pages_with_diff.entry(d).or_default().insert(i);
        }
        for a in result.agreed {
            *agreed.entry(a).or_default() += 1;
        }
    }
    let report = json!({
        "files": files.len(),
        "skipped": skipped,
        "debugBuildMs": {"renameScan": scan_ns / 1_000_000, "evidenceParse": evidence_ns / 1_000_000},
        "differences": diffs
            .iter()
            .map(|(d, n)| (d.clone(), json!({"occurrences": n, "pages": pages_with_diff[d].len()})))
            .collect::<BTreeMap<_, _>>(),
        "agreed": agreed,
    });
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../artifacts/cm");
    fs::create_dir_all(&out).unwrap();
    let text = serde_json::to_string_pretty(&report).unwrap();
    fs::write(out.join("extraction-anonymized.json"), &text).unwrap();
    eprintln!("{text}");
}
