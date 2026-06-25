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
fn code_ranges(raw: &str) -> Vec<std::ops::Range<usize>> {
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    let mut fence: Option<(u8, usize)> = None; // (marker byte, run length) while open
    let mut pos = 0usize;
    for line in raw.split_inclusive('\n') {
        let line_start = pos;
        pos += line.len();
        let content = line.strip_suffix('\n').unwrap_or(line);
        let trimmed = content.trim_start();
        let marker = trimmed.bytes().next().filter(|&c| c == b'`' || c == b'~');
        let run = marker.map_or(0, |m| trimmed.bytes().take_while(|&c| c == m).count());
        if let Some((fc, fl)) = fence {
            ranges.push(line_start..pos); // whole line (incl. newline) is code
            // A closing fence is the same marker, >= the opening run, nothing after it.
            if marker == Some(fc) && run >= fl && trimmed.as_bytes()[run..].iter().all(u8::is_ascii_whitespace) {
                fence = None;
            }
            continue;
        }
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
                    out.push(after[..end].to_string());
                    i += 2 + end + 2;
                    continue;
                }
            }
            if rest.starts_with('#') && tag_boundary(raw, i) {
                if let Some(after) = rest.strip_prefix("#[[") {
                    if let Some(end) = after.find("]]") {
                        out.push(after[..end].to_string());
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

/// Does `raw` reference page `target` (by `[[...]]` or `#tag`)?
pub fn references_page(raw: &str, target: &str) -> bool {
    let t = normalize(target);
    page_refs(raw).iter().any(|r| normalize(r) == t)
}

/// Rewrite every reference to page `from` (case-insensitive) as `to`, returning
/// the new text. Handles `[[from]]`, `#from`, and `#[[from]]`. A `#tag` becomes
/// `#[[to]]` when `to` contains characters that aren't valid in a bare tag
/// (e.g. spaces), matching Logseq.
pub fn rename_refs(raw: &str, from: &str, to: &str) -> String {
    let target = normalize(from);
    let code = code_ranges(raw);
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let rest = &raw[i..];
        // Inside a code fence / inline-code span, refs are literal — copy verbatim
        // (one char), never rewrite, so code examples aren't corrupted by a rename.
        if !in_code(i, &code) {
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
pub fn rename_tags_property(raw: &str, from: &str, to: &str) -> String {
    let target = normalize(from);
    let code = code_ranges(raw);
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
    fn extracts_block_refs() {
        assert_eq!(
            block_refs("ref ((628953c1-8d75-49fe-a648-f4c612109098)) here"),
            vec!["628953c1-8d75-49fe-a648-f4c612109098"]
        );
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
            rename_refs("visit https://ex.com/docs#Old and tag #Old", "Old", "New"),
            "visit https://ex.com/docs#Old and tag #New"
        );
    }

    #[test]
    fn rename_skips_refs_in_code() {
        // inline code is preserved verbatim; the prose ref is renamed
        assert_eq!(
            rename_refs("see [[Old]] and `[[Old]]`", "Old", "New"),
            "see [[New]] and `[[Old]]`"
        );
        // fenced code is preserved verbatim; refs outside are renamed
        let raw = "before [[Old]]\n```js\nconst x = \"[[Old]]\"; // #Old\n```\nafter #Old";
        let got = rename_refs(raw, "Old", "New");
        assert!(got.contains("before [[New]]"), "prose ref renamed: {got}");
        assert!(got.contains("after #New"), "trailing tag renamed: {got}");
        assert!(got.contains("\"[[Old]]\"; // #Old"), "code body untouched: {got}");
    }

    #[test]
    fn rename_tags_property_rewrites_bare_values_only() {
        // bare value matched, sibling + whitespace + commas preserved
        assert_eq!(
            rename_tags_property("tags:: Old, keep", "Old", "New"),
            "tags:: New, keep"
        );
        // case-insensitive match; original `to` casing used
        assert_eq!(rename_tags_property("tags:: old", "Old", "New"), "tags:: New");
        // bracketed / #-prefixed values are left for rename_refs (no double-rewrite)
        assert_eq!(
            rename_tags_property("tags:: [[Old]], #Old", "Old", "New"),
            "tags:: [[Old]], #Old"
        );
        // a non-tags property is untouched
        assert_eq!(rename_tags_property("author:: Old", "Old", "New"), "author:: Old");
        // a `tags::` line inside a code fence is literal — not rewritten
        let raw = "tags:: Old\n```\ntags:: Old\n```\n";
        assert_eq!(rename_tags_property(raw, "Old", "New"), "tags:: New\n```\ntags:: Old\n```\n");
    }
}
