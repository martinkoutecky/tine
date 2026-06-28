//! Reference extraction from block text: `[[page]]`, `#tag` (and `#[[multi
//! word]]`), and `((block-uuid))`. Used for the backlink index and queries.
//! UTF-8 safe (advances by char boundaries).

/// Normalize a page name for indexing/matching: trim + lowercase. Display uses
/// the original.
pub fn normalize(name: &str) -> String {
    name.trim().to_lowercase()
}

fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '_' | '/' | '.')
}

/// Byte ranges of `raw` that are inside code — fenced blocks (``` / ~~~) or
/// inline `…` spans. Like OG, references inside code are literal: they are
/// neither indexed as backlinks nor rewritten on rename, so a code example that
/// shows `[[Foo]]`/`#Foo` (or a URL fragment inside code) isn't corrupted when
/// page Foo is renamed. (A bare URL `…#Foo` in prose is a separate case.)
/// Strip one leading unordered-list bullet (`- `/`* `/`+ `) so a fenced code block
/// that opens directly on a bullet line (`- ```lang`) is recognized as a fence.
fn strip_list_bullet(s: &str) -> &str {
    let b = s.as_bytes();
    if b.len() >= 2 && matches!(b[0], b'-' | b'*' | b'+') && b[1] == b' ' {
        &s[2..]
    } else {
        s
    }
}

fn code_ranges(raw: &str) -> Vec<std::ops::Range<usize>> {
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    let mut fence: Option<(u8, usize)> = None; // (marker byte, run length) while open
    let mut pos = 0usize;
    for line in raw.split_inclusive('\n') {
        let line_start = pos;
        pos += line.len();
        let content = line.strip_suffix('\n').unwrap_or(line);
        let trimmed = content.trim_start();
        if let Some((fc, fl)) = fence {
            ranges.push(line_start..pos); // whole line (incl. newline) is code
            // A closing fence is the same marker, >= the opening run, nothing after
            // it. It is a bare line (no bullet) — a Logseq bulleted code block closes
            // with an aligned `  ``` `, so the close check uses the un-stripped text.
            let cm = trimmed.bytes().next().filter(|&c| c == b'`' || c == b'~');
            let cr = cm.map_or(0, |m| trimmed.bytes().take_while(|&c| c == m).count());
            if cm == Some(fc) && cr >= fl && trimmed.as_bytes()[cr..].iter().all(u8::is_ascii_whitespace) {
                fence = None;
            }
            continue;
        }
        // An OPENING fence may sit right after a list bullet (`- ```lang`), so strip
        // one bullet before testing. Without this, the opener is missed but its bare
        // closing ``` gets mis-read as an opener, swallowing everything after the
        // block (e.g. a later `[[ref]]`) as "code".
        let body = strip_list_bullet(trimmed);
        let marker = body.bytes().next().filter(|&c| c == b'`' || c == b'~');
        let run = marker.map_or(0, |m| body.bytes().take_while(|&c| c == m).count());
        if run >= 3 {
            ranges.push(line_start..pos);
            fence = Some((marker.unwrap(), run));
            continue;
        }
        inline_code_spans(content, line_start, &mut ranges);
    }
    ranges
}

/// Append byte ranges of inline `code` spans on one (non-fenced) line. A span is
/// a run of N backticks, closed by the next run of exactly N (CommonMark-ish).
fn inline_code_spans(line: &str, base: usize, out: &mut Vec<std::ops::Range<usize>>) {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'`' {
            let open = i;
            let k = b[i..].iter().take_while(|&&c| c == b'`').count();
            let mut j = i + k;
            let mut end = None;
            while j < b.len() {
                if b[j] == b'`' {
                    let r = b[j..].iter().take_while(|&&c| c == b'`').count();
                    if r == k {
                        end = Some(j + k);
                        break;
                    }
                    j += r;
                } else {
                    j += 1;
                }
            }
            match end {
                Some(e) => {
                    out.push(base + open..base + e);
                    i = e;
                }
                None => i = open + k, // unterminated: the backticks are literal
            }
        } else {
            i += 1;
        }
    }
}

fn in_code(pos: usize, ranges: &[std::ops::Range<usize>]) -> bool {
    ranges.iter().any(|r| r.contains(&pos))
}

/// Ranges to protect from ref rewriting: markdown fenced/inline code always, plus
/// — for an org file — `#+BEGIN_…#+END_…` blocks (whose `[[..]]`/`#..` are literal
/// source, not references). `is_org` is gated so a literal `#+BEGIN_` in a real
/// markdown file is never mistaken for a block.
fn code_ranges_for(raw: &str, is_org: bool) -> Vec<std::ops::Range<usize>> {
    let mut r = code_ranges(raw);
    if is_org {
        r.extend(org_block_ranges(raw));
    }
    r
}

/// Byte ranges (whole lines, inclusive) of org `#+BEGIN_x … #+END_x` blocks.
/// Mirrors `org.rs`'s headline-scanner block tracking; an unclosed block extends
/// to end-of-text (so a stray ref after it is treated conservatively as literal).
fn org_block_ranges(raw: &str) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut pos = 0usize;
    let mut depth = 0usize;
    let mut start = 0usize;
    for line in raw.split_inclusive('\n') {
        let line_start = pos;
        pos += line.len();
        let kw = line.trim_start_matches([' ', '\t']).strip_prefix("#+");
        let is_begin = kw.is_some_and(|k| k.len() >= 6 && k[..6].eq_ignore_ascii_case("begin_"));
        let is_end = kw.is_some_and(|k| k.len() >= 4 && k[..4].eq_ignore_ascii_case("end_"));
        if depth == 0 {
            if is_begin {
                depth = 1;
                start = line_start;
            }
        } else if is_begin {
            depth += 1;
        } else if is_end {
            depth -= 1;
            if depth == 0 {
                ranges.push(start..pos);
            }
        }
    }
    if depth > 0 {
        ranges.push(start..pos);
    }
    ranges
}

/// A `#tag` is only a tag at a word boundary: `#` at the start, or preceded by a
/// char that isn't itself tag-body material. So `word#x`, `ex.com#x`, `path/#x`
/// (URL fragments) are NOT tags — matching OG — while ` #x`, `(#x`, `]#x` are.
/// (`[[name]]` links don't need this; they're bracket-delimited.)
fn tag_boundary(raw: &str, i: usize) -> bool {
    i == 0 || raw[..i].chars().next_back().map_or(true, |c| !is_tag_char(c))
}

/// All page references in `raw`: `[[name]]`, `#tag`, and `#[[name]]`. Tags are
/// included because Logseq treats `#foo` as a reference to page `foo`.
pub fn page_refs(raw: &str) -> Vec<String> {
    let code = code_ranges(raw);
    let mut out = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let rest = &raw[i..];
        if !in_code(i, &code) {
            if let Some(after) = rest.strip_prefix("[[") {
                if let Some(end) = after.find("]]") {
                    // `[[]]` (empty) is not a page ref in OG — consume but don't index.
                    if !after[..end].trim().is_empty() {
                        out.push(after[..end].to_string());
                    }
                    i += 2 + end + 2;
                    continue;
                }
            }
            if rest.starts_with('#') && tag_boundary(raw, i) {
                if let Some(after) = rest.strip_prefix("#[[") {
                    if let Some(end) = after.find("]]") {
                        if !after[..end].trim().is_empty() {
                            out.push(after[..end].to_string());
                        }
                        i += 3 + end + 2;
                        continue;
                    }
                }
                let after = &rest[1..];
                let len = after.find(|c: char| !is_tag_char(c)).unwrap_or(after.len());
                if len > 0 {
                    out.push(after[..len].to_string());
                    i += 1 + len;
                    continue;
                }
            }
        }
        i += rest.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    }
    out
}

/// All block references `((uuid))` in `raw`.
pub fn block_refs(raw: &str) -> Vec<String> {
    let code = code_ranges(raw);
    let mut out = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let rest = &raw[i..];
        if !in_code(i, &code) {
            if let Some(after) = rest.strip_prefix("((") {
                if let Some(end) = after.find("))") {
                    out.push(after[..end].trim().to_string());
                    i += 2 + end + 2;
                    continue;
                }
            }
        }
        i += rest.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    }
    out
}

/// A block's `id::` property value (its uuid), if any.
pub fn block_id(raw: &str) -> Option<String> {
    raw.lines().find_map(|l| {
        crate::doc::parse_property_line(l)
            .and_then(|(k, v)| k.eq_ignore_ascii_case("id").then(|| v.trim().to_string()))
    })
}

/// Read a `[label](target)` starting at the leading `[`. The target is read with
/// BALANCED parens, so a URL that contains parens — `((uuid))`, `…/Foo_(bar)` — is
/// captured whole instead of stopping at the first `)`. Returns (label, target,
/// bytes consumed); only ASCII brackets are matched, so byte slicing is safe.
pub fn read_bracket_link(rest: &str) -> Option<(&str, &str, usize)> {
    let bytes = rest.as_bytes();
    if bytes.first() != Some(&b'[') {
        return None;
    }
    let label_end = rest.find(']')?;
    if bytes.get(label_end + 1) != Some(&b'(') {
        return None;
    }
    let url_start = label_end + 2;
    let mut depth = 1usize;
    let mut j = url_start;
    while j < bytes.len() {
        match bytes[j] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        j += 1;
    }
    if depth != 0 || j == url_start {
        return None;
    }
    Some((&rest[1..label_end], &rest[url_start..j], j + 1))
}

/// The inner uuid if `url` is exactly a `((uuid))` block-ref target.
pub fn as_block_ref(url: &str) -> Option<&str> {
    url.trim().strip_prefix("((").and_then(|s| s.strip_suffix("))")).map(str::trim)
}

/// A canonical block uuid (8-4-4-4-12 hex). OG only treats `((x))` as a block ref
/// when `x` parses as a UUID (`graph_parser/util/block_ref`), so a stray `((foo))`
/// in prose isn't counted as a reference or inflates a target's badge.
pub fn is_block_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 36 {
        return false;
    }
    b.iter().enumerate().all(|(i, &c)| match i {
        8 | 13 | 18 | 23 => c == b'-',
        _ => c.is_ascii_hexdigit(),
    })
}

/// Every block uuid `raw` references, deduped and code-aware. Covers all three OG
/// block-ref forms: bare `((uuid))`, labeled `[label](((uuid)))`, and
/// `{{embed ((uuid))}}` (the embed's `((uuid))` is caught by the bare scan). The
/// labeled form is consumed as a whole link FIRST — otherwise the bare-`((` scan
/// mis-parses its triple paren (capturing `(uuid` instead of `uuid`). Refs inside
/// code are ignored (literal), matching `block_refs`/`page_refs`.
pub fn block_ref_ids(raw: &str) -> Vec<String> {
    let code = code_ranges(raw);
    let mut out: Vec<String> = Vec::new();
    let push = |id: &str, out: &mut Vec<String>| {
        let id = id.trim();
        // Only a real UUID is a block ref (OG parse-uuid), so `((foo))` doesn't count.
        if is_block_uuid(id) && !out.iter().any(|x| x == id) {
            out.push(id.to_string());
        }
    };
    let mut i = 0;
    while i < raw.len() {
        let rest = &raw[i..];
        if !in_code(i, &code) {
            // Labeled `[label](((uuid)))` — consume the whole link first. A normal
            // `[text](url)` is consumed too (its url isn't a block ref → nothing
            // pushed), so the bare scan never peeks inside a markdown link's url.
            if rest.starts_with('[') {
                if let Some((_, url, len)) = read_bracket_link(rest) {
                    if let Some(id) = as_block_ref(url) {
                        push(id, &mut out);
                    }
                    i += len;
                    continue;
                }
            }
            // Bare `((uuid))` (also matches the body of `{{embed ((uuid))}}`).
            if let Some(after) = rest.strip_prefix("((") {
                if let Some(end) = after.find("))") {
                    push(&after[..end], &mut out);
                    i += 2 + end + 2;
                    continue;
                }
            }
        }
        i += rest.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    }
    out
}

/// Does `raw` reference page `target` (by `[[...]]` or `#tag`)?
pub fn references_page(raw: &str, target: &str) -> bool {
    let t = normalize(target);
    page_refs(raw).iter().any(|r| normalize(r) == t)
}

/// Rewrite every reference to page `from` (case-insensitive) as `to`, returning
/// the new text. Handles `[[from]]`, `#from`, and `#[[from]]`. A `#tag` becomes
/// `#[[to]]` when `to` contains characters that aren't valid in a bare tag
/// (e.g. spaces), matching Logseq.
pub fn rename_refs(raw: &str, from: &str, to: &str, is_org: bool) -> String {
    let target = normalize(from);
    let code = code_ranges_for(raw, is_org);
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let rest = &raw[i..];
        // Inside a code fence / inline-code span, refs are literal — copy verbatim
        // (one char), never rewrite, so code examples aren't corrupted by a rename.
        if !in_code(i, &code) {
            // Org file link: `[[file:…/<stem>.org][desc]]` / `[[file:…/<stem>.org]]`.
            // Its target is a path, not a `[[name]]`, so the generic handler below
            // can't match it — rewrite the filename stem so the link survives the
            // rename (L1). Only for org; markdown has no `file:` page links.
            if is_org && rest.starts_with("[[") {
                if let Some(end) = rest[2..].find("]]") {
                    if let Some(rw) = rewrite_org_file_link(&rest[2..2 + end], &target, to) {
                        out.push_str(&rw);
                        i += 2 + end + 2;
                        continue;
                    }
                }
            }
            if let Some(after) = rest.strip_prefix("[[") {
                if let Some(end) = after.find("]]") {
                    if normalize(&after[..end]) == target {
                        out.push_str(&format!("[[{to}]]"));
                    } else {
                        out.push_str(&raw[i..i + 2 + end + 2]);
                    }
                    i += 2 + end + 2;
                    continue;
                }
            }
            if tag_boundary(raw, i) {
                if let Some(after) = rest.strip_prefix("#[[") {
                    if let Some(end) = after.find("]]") {
                        if normalize(&after[..end]) == target {
                            out.push_str(&tag_for(to));
                        } else {
                            out.push_str(&raw[i..i + 3 + end + 2]);
                        }
                        i += 3 + end + 2;
                        continue;
                    }
                }
            }
            if rest.starts_with('#') && tag_boundary(raw, i) {
                let after = &rest[1..];
                let len = after.find(|c: char| !is_tag_char(c)).unwrap_or(after.len());
                if len > 0 {
                    if normalize(&after[..len]) == target {
                        out.push_str(&tag_for(to));
                    } else {
                        out.push_str(&raw[i..i + 1 + len]);
                    }
                    i += 1 + len;
                    continue;
                }
            }
        }
        let ch = rest.chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Rewrite an org `[[file:…]]` link's inner text if its target file's basename
/// (namespace-decoded `___`→`/`, extension stripped) normalizes to `target`.
/// Returns the full replacement `[[file:…]]` (preserving dir, extension, and any
/// `[desc]`), or `None` if it isn't a matching file link. Mirrors the model's
/// `encode_page_name` (`/`→`___`) so the new stem names the renamed file.
fn rewrite_org_file_link(inner: &str, target: &str, to: &str) -> Option<String> {
    let body = inner.strip_prefix("file:")?;
    let (path_part, desc) = match body.find("][") {
        Some(s) => (&body[..s], Some(&body[s + 2..])),
        None => (body, None),
    };
    let slash = path_part.rfind('/').map(|p| p + 1).unwrap_or(0);
    let (dir, file) = path_part.split_at(slash);
    let (stem, ext) = match file.rsplit_once('.') {
        Some((s, e)) => (s, format!(".{e}")),
        None => (file, String::new()),
    };
    if normalize(&stem.replace("___", "/")) != *target {
        return None;
    }
    let new_stem = to.replace('/', "___");
    let desc_part = desc.map(|d| format!("][{d}")).unwrap_or_default();
    Some(format!("[[file:{dir}{new_stem}{ext}{desc_part}]]"))
}

/// `#to` if `to` is a bare-tag-safe name, else `#[[to]]`.
fn tag_for(to: &str) -> String {
    if to.chars().all(is_tag_char) && !to.is_empty() {
        format!("#{to}")
    } else {
        format!("#[[{to}]]")
    }
}

/// Rewrite **bare** page-name refs in `tags::` property values from `from` to
/// `to`. `page_refs`/`rename_refs` only see inline `[[..]]`/`#..`, so bare
/// comma-separated tag names (`tags:: Old, Foo`) are invisible to them — yet
/// Logseq indexes those as real references, so a rename must update them too.
/// Bracketed (`[[..]]`) and `#`-prefixed values are left to `rename_refs`.
/// `tags::` lines inside a code fence are skipped (literal text, like inline
/// refs in code). Whitespace, commas, and the `key::` prefix are preserved
/// verbatim for byte-exact round-tripping of everything but the matched name.
pub fn rename_tags_property(raw: &str, from: &str, to: &str, is_org: bool) -> String {
    let target = normalize(from);
    let code = code_ranges_for(raw, is_org);
    let mut out = String::with_capacity(raw.len());
    let mut pos = 0usize;
    for line in raw.split_inclusive('\n') {
        let line_start = pos;
        pos += line.len();
        let content = line.strip_suffix('\n').unwrap_or(line);
        if !in_code(line_start, &code) {
            if let Some(vstart) = tags_value_start(content) {
                out.push_str(&content[..vstart]);
                out.push_str(&rewrite_bare_tags(&content[vstart..], &target, to));
                out.push_str(&line[content.len()..]); // trailing '\n', if any
                continue;
            }
        }
        out.push_str(line);
    }
    out
}

/// Byte offset where a `tags::` property line's value begins (just after `::`),
/// or `None` if `line` isn't a `tags::` property line.
fn tags_value_start(line: &str) -> Option<usize> {
    let (k, _) = crate::doc::parse_property_line(line)?;
    if !k.eq_ignore_ascii_case("tags") {
        return None;
    }
    line.find("::").map(|i| i + 2)
}

/// Rewrite a `tags::` value (the part after `::`): for each comma-separated
/// segment whose trimmed, **bare** name normalizes to `target`, swap the name
/// for `to`, keeping the segment's surrounding whitespace.
fn rewrite_bare_tags(valpart: &str, target: &str, to: &str) -> String {
    valpart
        .split(',')
        .map(|seg| {
            let trimmed = seg.trim();
            if trimmed.is_empty() || trimmed.starts_with("[[") || trimmed.starts_with('#') {
                return seg.to_string(); // empty, or handled by rename_refs
            }
            if normalize(trimmed) == *target {
                let lead = seg.len() - seg.trim_start().len();
                let trail = seg.trim_end().len();
                format!("{}{}{}", &seg[..lead], to, &seg[trail..])
            } else {
                seg.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_page_refs_and_tags() {
        let raw = "see [[Foo Bar]] and #tag and #[[multi word]] end";
        assert_eq!(page_refs(raw), vec!["Foo Bar", "tag", "multi word"]);
    }

    #[test]
    fn empty_brackets_are_not_refs() {
        // `[[]]` / `#[[]]` (empty) are not page refs in OG — consumed, not indexed;
        // a real ref afterward is still found.
        assert_eq!(page_refs("[[]]"), Vec::<String>::new());
        assert_eq!(page_refs("#[[]]"), Vec::<String>::new());
        assert_eq!(page_refs("[[ ]]"), Vec::<String>::new()); // whitespace-only
        assert_eq!(page_refs("[[]] then [[Real]]"), vec!["Real"]);
    }

    #[test]
    fn extracts_block_refs() {
        assert_eq!(
            block_refs("ref ((628953c1-8d75-49fe-a648-f4c612109098)) here"),
            vec!["628953c1-8d75-49fe-a648-f4c612109098"]
        );
    }

    #[test]
    fn block_ref_ids_all_forms_deduped_and_code_aware() {
        // Real UUIDs (block refs are only counted when the inner id is a UUID).
        let a = "11111111-1111-1111-1111-111111111111";
        let b = "22222222-2222-2222-2222-222222222222";
        let c = "33333333-3333-3333-3333-333333333333";
        let d = "44444444-4444-4444-4444-444444444444";
        let e = "55555555-5555-5555-5555-555555555555";
        // bare
        assert_eq!(block_ref_ids(&format!("see (({a})) end")), vec![a]);
        // labeled `[label](((uuid)))` — the triple paren must be captured whole
        assert_eq!(block_ref_ids(&format!("(see [Related Work]((({b})))) ")), vec![b]);
        // embed macro body
        assert_eq!(block_ref_ids(&format!("{{{{embed (({c}))}}}}")), vec![c]);
        // mixed + dedupe (same uuid twice → once)
        assert_eq!(
            block_ref_ids(&format!("(({d})) and [x]((({e}))) and (({d})) again")),
            vec![d, e]
        );
        // a normal markdown link is consumed whole, never mined for `((`
        assert_eq!(block_ref_ids("[text](https://ex.com/a)"), Vec::<String>::new());
        // refs inside code are literal → ignored
        assert_eq!(
            block_ref_ids(&format!("real (({a})) but `(({b}))` literal")),
            vec![a]
        );
        let fenced = format!("intro (({a}))\n```\n(({b}))\n```\nout (({c}))");
        assert_eq!(block_ref_ids(&fenced), vec![a, c]);
    }

    #[test]
    fn block_ref_ids_rejects_non_uuid() {
        // A `((word))` in prose is NOT a block ref (OG requires a UUID) — so it
        // doesn't inflate a count or create a phantom referrer.
        assert_eq!(block_ref_ids("see ((Related Work)) and ((foo))"), Vec::<String>::new());
        assert!(is_block_uuid("11111111-1111-1111-1111-111111111111"));
        assert!(!is_block_uuid("not-a-uuid"));
        assert!(!is_block_uuid("11111111111111111111111111111111"));
    }

    #[test]
    fn read_bracket_link_balances_parens() {
        // url with parens (block ref) captured whole, not stopped at first `)`
        assert_eq!(read_bracket_link("[L](((u)))rest"), Some(("L", "((u))", 10)));
        assert_eq!(as_block_ref("((u))"), Some("u"));
        // a paren-bearing normal url survives too
        assert_eq!(read_bracket_link("[a](/Foo_(bar)) x").map(|t| t.1), Some("/Foo_(bar)"));
        // not a link
        assert!(read_bracket_link("[a] (b)").is_none());
    }

    #[test]
    fn block_id_reads_id_property() {
        assert_eq!(block_id("text\nid:: 1234-abcd"), Some("1234-abcd".to_string()));
        assert_eq!(block_id("ID:: Xyz"), Some("Xyz".to_string())); // case-insensitive key
        assert_eq!(block_id("no props here"), None);
    }

    #[test]
    fn references_page_is_case_insensitive() {
        assert!(references_page("link to [[Logseq]]", "logseq"));
        assert!(references_page("a #project tag", "Project"));
        assert!(!references_page("no refs here", "logseq"));
    }

    #[test]
    fn utf8_safe() {
        let raw = "café [[naïve]] θ #tag";
        assert_eq!(page_refs(raw), vec!["naïve", "tag"]);
    }

    #[test]
    fn refs_in_code_are_ignored() {
        // inline code
        assert_eq!(page_refs("real [[Foo]] but `[[Foo]]` literal"), vec!["Foo"]);
        // fenced block (multi-line)
        let raw = "intro [[A]]\n```\nuse [[A]] and #A here\n```\noutro [[A]]";
        assert_eq!(page_refs(raw), vec!["A", "A"]); // the two outside the fence
        // a tag inside inline code isn't a ref
        assert_eq!(page_refs("`#nope` yes #yep"), vec!["yep"]);
    }

    #[test]
    fn hash_needs_a_word_boundary() {
        // bare-URL fragment is NOT a tag; the real tag after a space is
        assert_eq!(page_refs("see https://ex.com/p#Old then #real"), vec!["real"]);
        // glued to a word → not a tag
        assert_eq!(page_refs("a c#sharp note"), Vec::<String>::new());
        // legit boundaries still produce tags
        assert_eq!(page_refs("[[Foo]]#bar (#baz) ,#qux"), vec!["Foo", "bar", "baz", "qux"]);
    }

    #[test]
    fn rename_leaves_url_fragments_alone() {
        // `#Old` inside a URL isn't a tag → untouched; the real tag is renamed
        assert_eq!(
            rename_refs("visit https://ex.com/docs#Old and tag #Old", "Old", "New", false),
            "visit https://ex.com/docs#Old and tag #New"
        );
    }

    #[test]
    fn rename_skips_refs_in_code() {
        // inline code is preserved verbatim; the prose ref is renamed
        assert_eq!(
            rename_refs("see [[Old]] and `[[Old]]`", "Old", "New", false),
            "see [[New]] and `[[Old]]`"
        );
        // fenced code is preserved verbatim; refs outside are renamed
        let raw = "before [[Old]]\n```js\nconst x = \"[[Old]]\"; // #Old\n```\nafter #Old";
        let got = rename_refs(raw, "Old", "New", false);
        assert!(got.contains("before [[New]]"), "prose ref renamed: {got}");
        assert!(got.contains("after #New"), "trailing tag renamed: {got}");
        assert!(got.contains("\"[[Old]]\"; // #Old"), "code body untouched: {got}");
    }

    #[test]
    fn rename_rewrites_ref_after_bulleted_code_fence() {
        // Repro of the reported skip: a `[[ref]]` in a later bullet, AFTER a
        // ```calc fenced block that OPENS on a bullet line (`- ```calc`). The
        // bullet prefix used to hide the opener while its bare close (`  ``` `) was
        // mis-read as an opener, so the later ref looked "inside code" and was
        // skipped. The whole `## Tests` subtree mirrors Tine.md.
        let raw = "- ## Tests\n\t- ```calc\n\t  1 + 2\n\t  var = 2+4\n\t  ```\n\t- #+BEGIN_TIP\n\t  a tip\n\t  #+END_TIP\n\t- [[Pokus2]]\n";
        let out = rename_refs(raw, "Pokus2", "Pokus", false);
        assert!(out.contains("[[Pokus]]"), "ref after bulleted fence not rewritten: {out:?}");
        // the code block body itself is untouched
        assert!(out.contains("```calc") && out.contains("1 + 2"), "{out:?}");
    }

    #[test]
    fn rename_skips_refs_inside_org_begin_blocks() {
        // H2: with is_org=true, a `[[Old]]`/`#Old` literal inside an org
        // `#+BEGIN_SRC … #+END_SRC` block must NOT be rewritten (it's source text),
        // while a real ref outside the block still is.
        let raw = "see [[Old]] here\n#+BEGIN_SRC clojure\n(def s \"[[Old]]\") ; #Old\n#+END_SRC\nand [[Old]] again\n";
        let out = rename_refs(raw, "Old", "New", true);
        assert_eq!(
            out,
            "see [[New]] here\n#+BEGIN_SRC clojure\n(def s \"[[Old]]\") ; #Old\n#+END_SRC\nand [[New]] again\n"
        );
        // Same input as markdown (is_org=false) WOULD rewrite inside (no org fence
        // awareness) — proving the gate matters.
        let md = rename_refs(raw, "Old", "New", false);
        assert!(md.contains("(def s \"[[New]]\")"), "md path rewrites inside (expected): {md:?}");
    }

    #[test]
    fn rename_rewrites_org_file_links() {
        // L1: org `[[file:…/<stem>.org][desc]]` / `[[file:…]]` targets the renamed
        // file's stem — rewrite it (org only), preserving dir, extension, and desc.
        let raw = "[[file:./pages/Old.org][The Old]] and [[file:./pages/Old.org]] and [[Old]]\n";
        let out = rename_refs(raw, "Old", "New", true);
        assert_eq!(
            out,
            "[[file:./pages/New.org][The Old]] and [[file:./pages/New.org]] and [[New]]\n"
        );
        // Namespaced stem (`/`→`___`) and a non-matching file link are handled.
        assert_eq!(
            rename_refs("[[file:./pages/a___Old.org][x]]", "a/Old", "New/Sub", true),
            "[[file:./pages/New___Sub.org][x]]"
        );
        assert_eq!(
            rename_refs("[[file:./pages/Keep.org][k]]", "Old", "New", true),
            "[[file:./pages/Keep.org][k]]"
        );
        // Markdown (is_org=false) leaves file links alone (no org file-page links).
        assert_eq!(
            rename_refs("[[file:./pages/Old.org]]", "Old", "New", false),
            "[[file:./pages/Old.org]]"
        );
    }

    #[test]
    fn rename_tags_property_rewrites_bare_values_only() {
        // bare value matched, sibling + whitespace + commas preserved
        assert_eq!(
            rename_tags_property("tags:: Old, keep", "Old", "New", false),
            "tags:: New, keep"
        );
        // case-insensitive match; original `to` casing used
        assert_eq!(rename_tags_property("tags:: old", "Old", "New", false), "tags:: New");
        // bracketed / #-prefixed values are left for rename_refs (no double-rewrite)
        assert_eq!(
            rename_tags_property("tags:: [[Old]], #Old", "Old", "New", false),
            "tags:: [[Old]], #Old"
        );
        // a non-tags property is untouched
        assert_eq!(rename_tags_property("author:: Old", "Old", "New", false), "author:: Old");
        // a `tags::` line inside a code fence is literal — not rewritten
        let raw = "tags:: Old\n```\ntags:: Old\n```\n";
        assert_eq!(rename_tags_property(raw, "Old", "New", false), "tags:: New\n```\ntags:: Old\n```\n");
    }
}
