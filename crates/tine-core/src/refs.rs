//! Reference extraction from block text: `[[page]]`, `#tag` (and `#[[multi
//! word]]`), and `((block-uuid))`. Used for the backlink index and queries.
//! UTF-8 safe (advances by char boundaries).

use unicode_normalization::UnicodeNormalization;

use crate::config::FileNameFormat;

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

/// Pages whose text is never a reference source: the target page itself (OG
/// excludes a page from its own references) and Tine's Favorites arrangement
/// page (`:tine/favorites-page`), whose `[[links]]` are a sidebar layout, not a
/// mention. The ONE predicate; a new exclusion belongs here, not at a call site.
#[derive(Clone, Debug, Default)]
pub struct ReferenceSourceExclusions {
    keys: Vec<String>,
}

impl ReferenceSourceExclusions {
    pub fn new(self_page: &str, favorites_page: Option<&str>) -> Self {
        let mut keys = vec![page_key(self_page)];
        if let Some(key) = favorites_page.map(page_key) {
            if !key.is_empty() && !keys.contains(&key) {
                keys.push(key);
            }
        }
        Self { keys }
    }

    pub fn excludes_name(&self, page_name: &str) -> bool {
        let key = page_key(page_name);
        self.keys.iter().any(|candidate| *candidate == key)
    }
}

fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '_' | '/' | '.')
}

/// Whether byte `pos` is inside a code range, using a monotone cursor. The callers
/// (`rename_refs_multi`, `rename_tags_property`) scan left-to-right with a monotonically
/// increasing `pos`, and `ranges` are ascending + non-overlapping (see
/// `code_ranges_for`), so we advance `cursor` past spent ranges instead of scanning
/// ALL ranges for every byte — making rename O(n) instead of O(n·ranges).
fn in_code_at(pos: usize, ranges: &[std::ops::Range<usize>], cursor: &mut usize) -> bool {
    while *cursor < ranges.len() && ranges[*cursor].end <= pos {
        *cursor += 1;
    }
    ranges.get(*cursor).is_some_and(|r| r.contains(&pos))
}

/// Parser-owned literal bytes, including nested blocks and Org inline literals.
fn code_ranges_for(raw: &str, is_org: bool) -> Vec<std::ops::Range<usize>> {
    crate::block_regions::parse(raw, is_org)
        .literals
        .into_iter()
        .map(|r| r.0..r.1)
        .collect()
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
// with `publish.rs`), and parser-owned literal masks.

/// A block's `id::` property value (its uuid), if any.
pub fn block_id(raw: &str, is_org: bool) -> Option<String> {
    crate::block_regions::parse(raw, is_org)
        .id
        .map(|p| p.value.trim().to_string())
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

/// Rewrite every reference to ANY page in `renames` (keyed by `normalize(from)`,
/// valued by the display `to`) in a SINGLE left-to-right pass, computing the
/// code/fence ranges ONCE. This is the namespace-rename hot path: a primary page
/// with K file-backed descendants used to rescan every graph file K times (once
/// per `(old,new)` pair); now each file is scanned once against the whole rename
/// set. Each matched ref is mapped by its own normalized name (no chaining — a
/// reference to `A` always becomes `renames[A]`, even if some other pair renames
/// to `A`). Org `[[file:…]]` links decode and re-encode their target stem
/// through the graph's `file_name_format`, the codec the page move itself uses
/// (master b8f73b9af107), so legacy, triple-lowbar and reserved-character
/// targets name the renamed file.
pub fn rename_refs_multi(
    raw: &str,
    renames: &std::collections::HashMap<String, String>,
    is_org: bool,
    file_name_format: FileNameFormat,
) -> String {
    let code = code_ranges_for(raw, is_org);
    let mut code_cur = 0usize; // monotone cursor into `code` (i only increases)
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let rest = &raw[i..];
        // Inside a code fence / inline-code span, refs are literal — copy verbatim,
        // never rewrite, so code examples aren't corrupted by a rename.
        if !in_code_at(i, &code, &mut code_cur) {
            // Org file link: `[[file:…/<stem>.org][desc]]` / `[[file:…/<stem>.org]]`.
            // Its target is a path, not a `[[name]]`, so the generic handler below
            // can't match it — rewrite the filename stem so the link survives the
            // rename (L1). Only for org; markdown has no `file:` page links.
            if is_org && rest.starts_with("[[") {
                if let Some(end) = link_end(&rest[2..]) {
                    if let Some(rw) =
                        rewrite_org_file_link(&rest[2..2 + end], renames, file_name_format)
                    {
                        out.push_str(&rw);
                        i += 2 + end + 2;
                        continue;
                    }
                }
            }
            if let Some(after) = rest.strip_prefix("[[") {
                if let Some(end) = link_end(after) {
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
                    if let Some(end) = link_end(after) {
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
        // Only '[' and '#' can start a reference (master db17142fdf03, GH #406).
        // Copy the intervening literal run at once; the next opener still checks
        // its exact code-range position. Consume this character first so an
        // unmatched opener also makes progress.
        let first = rest.chars().next().unwrap().len_utf8();
        let end = first + rest[first..].find(['[', '#']).unwrap_or(rest.len() - first);
        out.push_str(&rest[..end]);
        i += end;
    }
    out
}

/// End of a `[[…]]` body (the offset of its first `]]`), unless another `[[`
/// opens before it. That opener is then not a link for rename: OG rewrites a
/// literal `[[Old]]` wherever it stands (`replace-page-ref!`), so the scan
/// resumes after one `[` and still finds `[[Old]]` in `use [[ to link [[Old]]`
/// and inside a nested `[[a [[Old]] c]]` (C3W W3, L03), where taking the span to
/// the first `]]` used to swallow the real reference and leave it stale.
fn link_end(after: &str) -> Option<usize> {
    let end = after.find("]]")?;
    (!after[..end].contains("[[")).then_some(end)
}

pub use crate::block_regions::is_linkable_property_separator;

/// Rewrite an org `[[file:…]]` link's inner text if its target file's basename
/// (namespace-decoded `___`→`/`, extension stripped) normalizes to a key in
/// `renames`. Returns the full replacement `[[file:…]]` (preserving dir,
/// extension, and any `[desc]`), or `None` if it isn't a matching file link.
/// Decodes and re-encodes through the graph's shared filename policy so legacy,
/// triple-lowbar, and reserved-character targets match the transactional move.
fn rewrite_org_file_link(
    inner: &str,
    renames: &std::collections::HashMap<String, String>,
    file_name_format: FileNameFormat,
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
    let decoded = crate::model::decode_page_name(stem, file_name_format);
    let to = renames.get(&normalize(&decoded))?;
    let new_stem = crate::model::encode_page_name(to, file_name_format);
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
/// `to`. `page_refs`/`rename_refs_multi` only see inline `[[..]]`/`#..`, so bare
/// comma-separated tag names (`tags:: Old, Foo`) are invisible to them — yet
/// Logseq indexes those as real references, so a rename must update them too.
/// Bracketed (`[[..]]`) and `#`-prefixed values are left to `rename_refs_multi`.
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
    // The property parser requires this literal separator.
    if !raw.contains("::") {
        return raw.to_owned();
    }
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
    let mut out = String::with_capacity(valpart.len());
    let mut rest = valpart;
    loop {
        let (seg, sep) = match rest.find(is_linkable_property_separator) {
            Some(at) => {
                let len = rest[at..].chars().next().map_or(1, char::len_utf8);
                (&rest[..at], Some(&rest[at..at + len]))
            }
            None => (rest, None),
        };
        let trimmed = seg.trim();
        let to = (!trimmed.is_empty() && !trimmed.starts_with("[[") && !trimmed.starts_with('#'))
            .then(|| renames.get(&normalize(trimmed)))
            .flatten(); // empty, or handled by rename_refs
        match to {
            Some(to) => {
                let lead = seg.len() - seg.trim_start().len();
                let trail = seg.trim_end().len();
                out.push_str(&seg[..lead]);
                out.push_str(to);
                out.push_str(&seg[trail..]);
            }
            None => out.push_str(seg),
        }
        match sep {
            Some(sep) => {
                out.push_str(sep);
                rest = &rest[seg.len() + sep.len()..];
            }
            None => return out,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Single-target rewrite: the one-entry case of [`rename_refs_multi`], so the
    /// tests exercise the same rewriter the graph rename uses.
    fn rename_refs(raw: &str, from: &str, to: &str, is_org: bool) -> String {
        let mut map = std::collections::HashMap::with_capacity(1);
        map.insert(normalize(from), to.to_string());
        rename_refs_multi(raw, &map, is_org, FileNameFormat::TripleLowbar)
    }

    #[test]
    fn rename_literal_runs_cross_code_boundaries_and_preserve_unicode() {
        let literal = "é猫 ordinary prose ".repeat(100);
        let raw = format!("{literal}`literal code` [[Old]]\n```\n{literal}[[Old]]\n```\n{literal}#Old [broken [ [[Old]]");
        let expected = format!("{literal}`literal code` [[New]]\n```\n{literal}[[Old]]\n```\n{literal}#New [broken [ [[New]]");
        assert_eq!(rename_refs(&raw, "Old", "New", false), expected);
        let org = format!("{literal}\n#+BEGIN_SRC\n[[Old]]\n#+END_SRC\n{literal}[[Old]]");
        assert_eq!(
            rename_refs(&org, "Old", "New", true),
            format!("{literal}\n#+BEGIN_SRC\n[[Old]]\n#+END_SRC\n{literal}[[New]]")
        );
    }

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
            block_id("text\nid:: 1234-abcd", false),
            Some("1234-abcd".to_string())
        );
        assert_eq!(block_id("ID:: Xyz", false), Some("Xyz".to_string())); // case-insensitive key
        assert_eq!(block_id("no props here", false), None);
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
        // mldoc 1.5.9 emits Src for this input in Markdown too. The same
        // parser-owned literal protection applies in both formats.
        let md = rename_refs(raw, "Old", "New", false);
        assert_eq!(md, out);
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
    fn org_file_link_rename_uses_safe_page_filename_codec() {
        for (format, old_title, old_stem, new_title) in [
            (
                FileNameFormat::Legacy,
                "Old.Name",
                "Old%2EName",
                "2026-07-23_18:01:20",
            ),
            (
                FileNameFormat::TripleLowbar,
                "Old/Name",
                "Old___Name",
                "CON",
            ),
        ] {
            let mut renames = std::collections::HashMap::new();
            renames.insert(normalize(old_title), new_title.to_owned());
            let raw = format!("[[file:./pages/{old_stem}.org][page]]");
            let expected = format!(
                "[[file:./pages/{}.org][page]]",
                crate::model::encode_page_name(new_title, format)
            );
            assert_eq!(rename_refs_multi(&raw, &renames, true, format), expected);
        }
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
