//! Reference extraction from block text: `[[page]]`, `#tag` (and `#[[multi
//! word]]`), and `((block-uuid))`. Used for the backlink index and queries.
//! UTF-8 safe (advances by char boundaries).

use unicode_normalization::UnicodeNormalization;

/// The ONE page-name identity key: trimmed + **Unicode** lowercase + NFC (the
/// OG/Logseq fold). Use this — never a bare
/// `to_ascii_lowercase`/`eq_ignore_ascii_case` on a
/// page name — so the ref/backlink index and the file/cache resolution agree on
/// identity (a non-ASCII name like `Über` must resolve the same everywhere). Display
/// uses the original casing.
pub fn page_key(name: &str) -> String {
    // Preserve Tine's historical surrounding-whitespace tolerance. Otherwise
    // this is OG page-name-sanity-lc: lowercase, remove one slash at each
    // boundary, then NFC (never NFKC or accent folding).
    let lowered = name.trim().to_lowercase();
    let without_leading = lowered.strip_prefix('/').unwrap_or(&lowered);
    let without_boundaries = without_leading.strip_suffix('/').unwrap_or(without_leading);
    without_boundaries.nfc().collect()
}

/// Comparison form for page identity. NFC composition requires allocation; this
/// deliberately delegates to the canonical key so cache scans cannot drift.
pub fn same_page(a: &str, b: &str) -> bool {
    page_key(a) == page_key(b)
}

/// Historical name for the page-identity fold used throughout ref extraction;
/// identical to [`page_key`] (kept so existing ref code reads naturally).
pub fn normalize(name: &str) -> String {
    page_key(name)
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
            if cm == Some(fc)
                && cr >= fl
                && trimmed.as_bytes()[cr..].iter().all(u8::is_ascii_whitespace)
            {
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

/// Whether byte `pos` is inside a code range, using a monotone cursor. The callers
/// (`rename_refs`, `rename_tags_property`) scan left-to-right with a monotonically
/// increasing `pos`, and `ranges` are ascending + non-overlapping (see
/// `code_ranges_for`), so we advance `cursor` past spent ranges instead of scanning
/// ALL ranges for every byte — making rename O(n) instead of O(n·ranges).
fn in_code_at(pos: usize, ranges: &[std::ops::Range<usize>], cursor: &mut usize) -> bool {
    while *cursor < ranges.len() && ranges[*cursor].end <= pos {
        *cursor += 1;
    }
    ranges.get(*cursor).is_some_and(|r| r.contains(&pos))
}

/// Ranges to protect from ref rewriting: markdown fenced/inline code always, plus
/// — for an org file — `#+BEGIN_…#+END_…` blocks (whose `[[..]]`/`#..` are literal
/// source, not references). `is_org` is gated so a literal `#+BEGIN_` in a real
/// markdown file is never mistaken for a block.
fn code_ranges_for(raw: &str, is_org: bool) -> Vec<std::ops::Range<usize>> {
    let mut r = code_ranges(raw);
    if is_org {
        // `code_ranges` is already ascending+non-overlapping; `org_block_ranges` is
        // appended out of byte-order, so re-sort + coalesce to restore the invariant
        // the monotone-cursor `in_code_at` relies on. R = #code regions (tiny), and
        // this runs once per rename — the per-byte scan stays O(n).
        r.extend(org_block_ranges(raw));
        r.sort_unstable_by_key(|x| x.start);
        let mut merged: Vec<std::ops::Range<usize>> = Vec::with_capacity(r.len());
        for cur in r {
            match merged.last_mut() {
                Some(prev) if cur.start <= prev.end => prev.end = prev.end.max(cur.end),
                _ => merged.push(cur),
            }
        }
        return merged;
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
    i == 0
        || raw[..i]
            .chars()
            .next_back()
            .map_or(true, |c| !is_tag_char(c))
}

// NOTE: the OG-faithful page/block ref EXTRACTORS live in lsdoc (see
// `render::block_refs` → `doc.rs` `projection()`, consumed by every query/backlink).
// The hand-rolled `page_refs`/`block_refs`/`block_ref_ids`/`references_page` that
// used to sit here were a dead second copy (only tests called them) — a "fix the
// wrong file" trap — and were removed. What remains in this file is the LIVE half:
// `normalize`, `rename_*`, `block_id`, the bracket-link/block-ref helpers (shared
// with `publish.rs`), and the code/org fence machinery.

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
    url.trim()
        .strip_prefix("((")
        .and_then(|s| s.strip_suffix("))"))
        .map(str::trim)
}

/// Rewrite every reference to page `from` (case-insensitive) as `to`, returning
/// the new text. Handles `[[from]]`, `#from`, and `#[[from]]`. A `#tag` becomes
/// `#[[to]]` when `to` contains characters that aren't valid in a bare tag
/// (e.g. spaces), matching Logseq.
pub fn rename_refs(raw: &str, from: &str, to: &str, is_org: bool) -> String {
    // Single-target is just the one-entry multi case — keep ONE rewriter so the
    // single- and multi-target callers can never drift on matching/escaping rules.
    let mut map = std::collections::HashMap::with_capacity(1);
    map.insert(normalize(from), to.to_string());
    rename_refs_multi(raw, &map, is_org)
}

/// Rewrite every reference to ANY page in `renames` (keyed by `normalize(from)`,
/// valued by the display `to`) in a SINGLE left-to-right pass, computing the
/// code/fence ranges ONCE. This is the namespace-rename hot path: a primary page
/// with K file-backed descendants used to rescan every graph file K times (once
/// per `(old,new)` pair); now each file is scanned once against the whole rename
/// set. Each matched ref is mapped by its own normalized name (no chaining — a
/// reference to `A` always becomes `renames[A]`, even if some other pair renames
/// to `A`).
pub fn rename_refs_multi(
    raw: &str,
    renames: &std::collections::HashMap<String, String>,
    is_org: bool,
) -> String {
    let code = code_ranges_for(raw, is_org);
    let mut code_cur = 0usize; // monotone cursor into `code` (i only increases)
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let rest = &raw[i..];
        // Inside a code fence / inline-code span, refs are literal — copy verbatim
        // (one char), never rewrite, so code examples aren't corrupted by a rename.
        if !in_code_at(i, &code, &mut code_cur) {
            // Org file link: `[[file:…/<stem>.org][desc]]` / `[[file:…/<stem>.org]]`.
            // Its target is a path, not a `[[name]]`, so the generic handler below
            // can't match it — rewrite the filename stem so the link survives the
            // rename (L1). Only for org; markdown has no `file:` page links.
            if is_org && rest.starts_with("[[") {
                if let Some(end) = rest[2..].find("]]") {
                    if let Some(rw) = rewrite_org_file_link(&rest[2..2 + end], renames) {
                        out.push_str(&rw);
                        i += 2 + end + 2;
                        continue;
                    }
                }
            }
            if let Some(after) = rest.strip_prefix("[[") {
                if let Some(end) = after.find("]]") {
                    if let Some(to) = renames.get(&normalize(&after[..end])) {
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
                        if let Some(to) = renames.get(&normalize(&after[..end])) {
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
                    if let Some(to) = renames.get(&normalize(&after[..len])) {
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
/// (namespace-decoded `___`→`/`, extension stripped) normalizes to a key in
/// `renames`. Returns the full replacement `[[file:…]]` (preserving dir,
/// extension, and any `[desc]`), or `None` if it isn't a matching file link.
/// Mirrors the model's `encode_page_name` (`/`→`___`) so the new stem names the
/// renamed file.
fn rewrite_org_file_link(
    inner: &str,
    renames: &std::collections::HashMap<String, String>,
) -> Option<String> {
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
    let to = renames.get(&normalize(&stem.replace("___", "/")))?;
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
    let mut map = std::collections::HashMap::with_capacity(1);
    map.insert(normalize(from), to.to_string());
    rename_tags_property_multi(raw, &map, is_org)
}

/// Multi-target `rename_tags_property`: rewrite bare `tags::` values that
/// normalize to ANY key in `renames` in a single pass (code ranges computed once).
/// The namespace-rename companion to [`rename_refs_multi`].
pub fn rename_tags_property_multi(
    raw: &str,
    renames: &std::collections::HashMap<String, String>,
    is_org: bool,
) -> String {
    let code = code_ranges_for(raw, is_org);
    let mut code_cur = 0usize; // monotone cursor (line_start only increases)
    let mut out = String::with_capacity(raw.len());
    let mut pos = 0usize;
    for line in raw.split_inclusive('\n') {
        let line_start = pos;
        pos += line.len();
        let content = line.strip_suffix('\n').unwrap_or(line);
        if !in_code_at(line_start, &code, &mut code_cur) {
            if let Some(vstart) = tags_value_start(content) {
                out.push_str(&content[..vstart]);
                out.push_str(&rewrite_bare_tags(&content[vstart..], renames));
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
fn rewrite_bare_tags(valpart: &str, renames: &std::collections::HashMap<String, String>) -> String {
    valpart
        .split(',')
        .map(|seg| {
            let trimmed = seg.trim();
            if trimmed.is_empty() || trimmed.starts_with("[[") || trimmed.starts_with('#') {
                return seg.to_string(); // empty, or handled by rename_refs
            }
            if let Some(to) = renames.get(&normalize(trimmed)) {
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
    fn read_bracket_link_balances_parens() {
        // url with parens (block ref) captured whole, not stopped at first `)`
        assert_eq!(
            read_bracket_link("[L](((u)))rest"),
            Some(("L", "((u))", 10))
        );
        assert_eq!(as_block_ref("((u))"), Some("u"));
        // a paren-bearing normal url survives too
        assert_eq!(
            read_bracket_link("[a](/Foo_(bar)) x").map(|t| t.1),
            Some("/Foo_(bar)")
        );
        // not a link
        assert!(read_bracket_link("[a] (b)").is_none());
    }

    #[test]
    fn block_id_reads_id_property() {
        assert_eq!(
            block_id("text\nid:: 1234-abcd"),
            Some("1234-abcd".to_string())
        );
        assert_eq!(block_id("ID:: Xyz"), Some("Xyz".to_string())); // case-insensitive key
        assert_eq!(block_id("no props here"), None);
    }

    #[test]
    fn rename_leaves_url_fragments_alone() {
        // `#Old` inside a URL isn't a tag → untouched; the real tag is renamed
        assert_eq!(
            rename_refs(
                "visit https://ex.com/docs#Old and tag #Old",
                "Old",
                "New",
                false
            ),
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
        assert!(
            got.contains("\"[[Old]]\"; // #Old"),
            "code body untouched: {got}"
        );
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
        assert!(
            out.contains("[[Pokus]]"),
            "ref after bulleted fence not rewritten: {out:?}"
        );
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
        assert!(
            md.contains("(def s \"[[New]]\")"),
            "md path rewrites inside (expected): {md:?}"
        );
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
    fn page_key_folds_case_only_never_diacritics() {
        // Case-variants of the SAME name fold together (one page, OG behavior)...
        assert!(same_page("Über", "über"));
        assert!(same_page("  Foo Bar ", "foo bar")); // trims too
        assert_eq!(page_key("Über"), page_key("über"));
        // ...but diacritics are NEVER stripped: distinct names stay distinct pages.
        assert!(!same_page("Uber", "Über")); // u != ü
        assert_ne!(page_key("Uber"), page_key("Über"));
        assert!(!same_page("Cafe", "Café"));
        assert!(same_page("Café", "Cafe\u{301}"));
        assert_eq!(page_key("/CAFÉ/"), page_key("Cafe\u{301}"));
        assert_ne!(page_key("Σ"), page_key("S")); // Greek sigma is not Latin S
                                                  // normalize is the same fold as page_key (single source).
        assert_eq!(normalize("Über"), page_key("Über"));
        // `str::to_lowercase` applies Unicode's contextual final-sigma rule.
        // The frontend navigation key mirrors this exact result.
        assert_eq!(page_key(" ΟΣ "), "ος");
    }

    #[test]
    fn rename_monotone_cursor_handles_many_interleaved_code_spans() {
        // 3 real refs (renamed) interleaved with 2 inline-code spans (literal). The
        // O(n) monotone cursor must advance past each spent code span without losing
        // a later real ref or wrongly rewriting one inside code.
        let raw = "[[Old]] `[[Old]]` mid [[Old]] `x [[Old]]` end [[Old]]";
        assert_eq!(
            rename_refs(raw, "Old", "New", false),
            "[[New]] `[[Old]]` mid [[New]] `x [[Old]]` end [[New]]"
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
        assert_eq!(
            rename_tags_property("tags:: old", "Old", "New", false),
            "tags:: New"
        );
        // bracketed / #-prefixed values are left for rename_refs (no double-rewrite)
        assert_eq!(
            rename_tags_property("tags:: [[Old]], #Old", "Old", "New", false),
            "tags:: [[Old]], #Old"
        );
        // a non-tags property is untouched
        assert_eq!(
            rename_tags_property("author:: Old", "Old", "New", false),
            "author:: Old"
        );
        // a `tags::` line inside a code fence is literal — not rewritten
        let raw = "tags:: Old\n```\ntags:: Old\n```\n";
        assert_eq!(
            rename_tags_property(raw, "Old", "New", false),
            "tags:: New\n```\ntags:: Old\n```\n"
        );
    }
}
