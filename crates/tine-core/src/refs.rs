//! Reference extraction from block text: `[[page]]`, `#tag` (and `#[[multi
//! word]]`), and `((block-uuid))`. Used for the backlink index and queries.
//! UTF-8 safe (advances by char boundaries).

use crate::config::FileNameFormat;

#[path = "page_identity.rs"]
pub(crate) mod page_identity;
pub use page_identity::page_key;

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

/// Whether `name` is a namespace name: OG `text.cljs namespace-page?`
/// (`deps/graph-parser/src/logseq/graph_parser/text.cljs:79`, logseq/logseq
/// 6e7afa8eb). It contains `/`, does not start with `./` or `../`, and is not a
/// URL (`util.cljs url?`: `new URL(s)` parses and its origin is not `"null"`,
/// which holds for `http`, `https`, `ws`, `wss`, `ftp` and a `blob:` of one of
/// them, not for `mailto:`, `file:` or an unknown scheme). Every consumer of
/// "which namespace pages does this name give" asks this, through
/// [`namespace_refs`]: a page's namespace parents, Linked References' namespace
/// closure, the queries' namespace closure; and directly the rename's moved
/// subtree and own-child guard (model round 10, S15) and the inventory wire's
/// `not_namespace` flag the namespace views read. The URL parse runs only for
/// a name with both `/` and `:`.
pub fn is_namespace_name(name: &str) -> bool {
    name.contains('/')
        && !name.starts_with("../")
        && !name.starts_with("./")
        && !url_has_origin(name)
}

/// OG `gp-util/url?` (`util.cljs:82`): `name` parses as a WHATWG URL whose
/// origin is not opaque (`"null"`). Only [`is_namespace_name`] asks it (and the
/// conformance harness, which hands it to the model as `urlHasOrigin`). Cheap
/// for a name without `:` (no parse).
pub fn url_has_origin(name: &str) -> bool {
    name.contains(':') && url::Url::parse(name).is_ok_and(|parsed| parsed.origin().is_tuple())
}

/// OG's page-reference closure: the name and each nonempty namespace ancestor
/// (only the name itself when it is not a namespace name,
/// [`is_namespace_name`]). Inputs retain their spelling; callers fold page
/// identity before comparing. Work and output depend only on this name, never
/// on graph size.
pub fn namespace_refs(name: &str) -> impl Iterator<Item = &str> {
    let prefixes = if is_namespace_name(name) { name } else { "" };
    prefixes
        .match_indices('/')
        .map(move |(at, _)| &name[..at])
        .chain(std::iter::once(name))
        .filter(|name| !name.is_empty())
}

/// The renamed spelling of a page name, including unclaimed namespace
/// members. `pages` maps old page keys to new display names. Shared by the
/// text rewriter and the journal-reference-loss guard; O(namespace depth).
pub fn renamed_page_name<'a>(
    name: &str,
    pages: &'a std::collections::HashMap<String, String>,
) -> Option<std::borrow::Cow<'a, str>> {
    if let Some(to) = pages.get(&page_key(name)) {
        return Some(std::borrow::Cow::Borrowed(to));
    }
    namespace_refs(name)
        .filter(|prefix| prefix.len() < name.len())
        .filter_map(|prefix| pages.get(&page_key(prefix)).map(|to| (prefix, to)))
        .last()
        .map(|(prefix, to)| std::borrow::Cow::Owned(format!("{to}{}", &name[prefix.len()..])))
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

// NOTE: the OG-faithful page/block ref EXTRACTORS live in lsdoc (see
// `render::block_refs` → `doc.rs` `projection()`, consumed by every query/backlink).
// The hand-rolled `page_refs`/`block_refs`/`block_ref_ids`/`references_page` that
// used to sit here were a dead second copy (only tests called them) — a "fix the
// wrong file" trap — and were removed. What remains in this file is the LIVE half:
// `normalize`, `rename_*` (whose sites come from the same evidence, `refs_rename.rs`),
// `block_id` and the bracket-link/block-ref helpers.

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
#[cfg(test)]
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
#[cfg(test)]
pub fn as_block_ref(url: &str) -> Option<&str> {
    url.trim()
        .strip_prefix("((")
        .and_then(|s| s.strip_suffix("))"))
        .map(str::trim)
}

pub use crate::block_regions::is_linkable_property_separator;

#[path = "refs_rename.rs"]
mod rename;

/// The one reference rewrite a page rename applies to a page's text: every
/// place the reference evidence says names a renamed page (links, tags,
/// embeds, nested-link names, linkable property members, property keys) and
/// every page-name argument of a query or macro, respelled for the new name;
/// every other byte is kept ([`rename`] has the classes and the declared OG
/// differences). Keys are raw old names, normalized here; values are display
/// names. The store's rename steps and the rename planner both call this, so
/// they cannot disagree on whether a rename changes a file. Pure, O(text).
pub fn rename_rewrite(
    text: &str,
    is_org: bool,
    renames: &[(String, String)],
    file_name_format: FileNameFormat,
) -> String {
    rename_rewrite_with_queries(text, is_org, renames, &[], file_name_format, None)
}

/// [`rename_rewrite`] where a query's page-name arguments naming a key of
/// `queries` take that name instead (a merge through an alias writes the
/// survivor's own name there, ruling (a) of D-20261103-CMA; links and
/// namespace children keep `renames`' typed name). Empty `queries` is
/// [`rename_rewrite`].
/// If supplied, `rewritten` receives distinct old/new logical-name pairs of edits
/// actually spliced into the text (including macro/query/file-link targets).
/// Rejected, overlapping and byte-identical edits contribute nothing. Recording
/// costs O(name bytes) per proposed edit and retains unique pairs, with no
/// additional parse or I/O.
pub fn rename_rewrite_with_queries(
    text: &str,
    is_org: bool,
    renames: &[(String, String)],
    queries: &[(String, String)],
    file_name_format: FileNameFormat,
    rewritten: Option<&mut std::collections::HashSet<(String, String)>>,
) -> String {
    let keyed = |pairs: &[(String, String)]| -> std::collections::HashMap<String, String> {
        pairs
            .iter()
            .map(|(from, to)| (normalize(from), to.clone()))
            .collect()
    };
    let renames = keyed(renames);
    let queries = keyed(queries);
    rename::rewrite_page(
        text,
        is_org,
        &rename::Renames::new(
            &renames,
            Some(&queries),
            file_name_format,
            rewritten.is_some(),
        ),
        rewritten,
    )
}

pub use rename::{RenameProposal, Site, Unrealized};

/// Native spelling of one linkable-property member, sharing the span writer's
/// wrapper rule so commas inside a name do not become member separators.
pub fn linkable_member_spelling(name: &str) -> String {
    rename::member_spelling(name)
}

/// Physical block ranges from the same outline authority as the page reader.
/// Each tuple is (whole block, content start, structural level, continuation
/// indentation). Ranges include line terminators; children follow their parent.
/// O(page bytes), one outline parse, no I/O.
pub fn physical_outline(
    text: &str,
    is_org: bool,
) -> Vec<(std::ops::Range<usize>, usize, u32, usize)> {
    let format = if is_org {
        crate::outline::OutlineFormat::Org
    } else {
        crate::outline::OutlineFormat::Markdown
    };
    let headers = crate::outline::headers(text, format).unwrap_or_default();
    let mut starts = vec![0];
    starts.extend(text.match_indices('\n').map(|(at, _)| at + 1));
    headers
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let start = starts[h.line];
            let end = headers.get(i + 1).map_or(text.len(), |h| starts[h.line]);
            (start..end, start + h.prefix_len, h.level, h.content_indent)
        })
        .collect()
}

/// Prepare the physical span rewrite with its checked complete-text certificate
/// and covered sites that could not be realized. O(page bytes), no I/O.
pub fn rename_proposal(
    text: &str,
    is_org: bool,
    renames: &std::collections::HashMap<String, String>,
    queries: &std::collections::HashMap<String, String>,
    format: FileNameFormat,
) -> Result<RenameProposal, crate::text_trace::TraceError> {
    rename::proposal(
        text,
        is_org,
        &rename::Renames::new(renames, Some(queries), format, true),
    )
}

/// [`rename_rewrite`] with the renames already keyed by `normalize(old)`.
pub fn rename_rewrite_keyed(
    text: &str,
    is_org: bool,
    renames: &std::collections::HashMap<String, String>,
    file_name_format: FileNameFormat,
) -> String {
    rename::rewrite_page(
        text,
        is_org,
        &rename::Renames::new(renames, None, file_name_format, false),
        None,
    )
}

/// [`rename_rewrite_keyed`] for one held block's raw body;
/// `queries` as [`rename_rewrite_with_queries`].
/// `rewritten` records applied edits as in that function.
pub fn rename_block_raw(
    raw: &str,
    is_org: bool,
    renames: &std::collections::HashMap<String, String>,
    queries: Option<&std::collections::HashMap<String, String>>,
    file_name_format: FileNameFormat,
    rewritten: Option<&mut std::collections::HashSet<(String, String)>>,
) -> String {
    rename::rewrite_block(
        raw,
        is_org,
        &rename::Renames::new(renames, queries, file_name_format, rewritten.is_some()),
        rewritten,
    )
}

/// [`rename_rewrite_keyed`] for a held page DTO's `pre_block`: its page
/// properties; `queries` as
/// [`rename_rewrite_with_queries`].
/// `rewritten` records applied edits as in that function.
pub fn rename_pre_block(
    pre: &str,
    is_org: bool,
    renames: &std::collections::HashMap<String, String>,
    queries: Option<&std::collections::HashMap<String, String>>,
    file_name_format: FileNameFormat,
    rewritten: Option<&mut std::collections::HashSet<(String, String)>>,
) -> String {
    rename::rewrite_pre_block(
        pre,
        is_org,
        &rename::Renames::new(renames, queries, file_name_format, rewritten.is_some()),
        rewritten,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CM step 2 batch 14 (S8 follow-up): OG `namespace-page?` excludes `./x`,
    /// `../x` and URLs, so such a name gives no namespace parents and no
    /// namespace-closure reference (`[[./x]]` is not a reference to page `.`).
    #[test]
    fn namespace_refs_follow_og_namespace_page() {
        fn refs(name: &str) -> Vec<&str> {
            namespace_refs(name).collect()
        }
        // Control: an ordinary namespaced name, and one with no `/`.
        assert_eq!(refs("a/b"), ["a", "a/b"]);
        assert_eq!(refs("a/b/c"), ["a", "a/b", "a/b/c"]);
        assert_eq!(refs("plain"), ["plain"]);
        assert_eq!(refs("a/./b"), ["a", "a/.", "a/./b"]);
        assert_eq!(refs(".hidden/x"), [".hidden", ".hidden/x"]);
        assert_eq!(refs("/lead"), ["/lead"]);
        // Excluded: relative paths and URLs are plain names.
        for name in [
            "./x",
            "./x/y",
            "../x",
            "../x/y",
            "https://a.b/c",
            "HTTPS://a.b/c/d",
            "http://x/y/z",
            "ftp://h/p",
            "ws://h/p",
            "wss://h/p",
            "blob:https://a.b/uuid",
            "https:/x/y",
        ] {
            assert_eq!(refs(name), [name], "{name}");
            assert!(!is_namespace_name(name), "{name}");
        }
        // OG's `url?` tests the origin, so these are namespaces.
        for name in [
            "mailto:a@b/c",
            "Work: Plan/x",
            "file:///a/b",
            "a://b/c",
            "tel:1/2",
            "C:/x/y",
            "blob:foo/bar",
        ] {
            assert!(is_namespace_name(name), "{name}");
            assert!(refs(name).len() > 1, "{name}");
        }
    }

    /// Single-target rewrite of a page's text: the one-entry case of
    /// [`rename_rewrite`], the rewriter the graph rename uses.
    fn rename_refs(raw: &str, from: &str, to: &str, is_org: bool) -> String {
        rename_rewrite(
            raw,
            is_org,
            &[(from.to_owned(), to.to_owned())],
            FileNameFormat::TripleLowbar,
        )
    }

    /// The same rewrite of one block's body (the text after its bullet).
    fn rename_in_block(raw: &str, from: &str, to: &str, is_org: bool) -> String {
        let mut map = std::collections::HashMap::with_capacity(1);
        map.insert(normalize(from), to.to_string());
        rename_block_raw(raw, is_org, &map, None, FileNameFormat::TripleLowbar, None)
    }

    /// The rewrite of a page whose text starts with page properties.
    fn rename_tags_property(raw: &str, from: &str, to: &str, is_org: bool) -> String {
        rename_refs(raw, from, to, is_org)
    }

    #[test]
    fn audit_l3_02_rename_uses_complete_parser_tag_tokens() {
        let cases = [
            ("#foo+bar", "foo", "#foo+bar"),
            ("#c++", "c", "#c++"),
            ("#c++", "c++", "#New"),
            ("#R&D", "R", "#R&D"),
            ("#foo🎉", "foo", "#foo🎉"),
            ("#foo.", "foo", "#New."),
            ("#foo...", "foo", "#New..."),
            ("#foo,", "foo", "#New,"),
            // Intentional OG difference: lsdoc also excludes these trailing
            // delimiters, while OG's replace-tag-ref! only accepts commas/dots.
            ("#foo;", "foo", "#New;"),
            ("#foo!", "foo", "#New!"),
            ("#foo.bar", "foo", "#foo.bar"),
            ("#foo.bar", "foo.bar", "#New"),
            ("#foo", "foo", "#New"),
            ("[[foo]] and #foo+bar", "foo", "[[New]] and #foo+bar"),
        ];
        let mut failures = Vec::new();
        for is_org in [false, true] {
            let bullet = if is_org { "* " } else { "- " };
            for (tag, from, expected) in cases {
                // Both EOL and whitespace after the token/delimiter run.
                for suffix in ["", " next\n"] {
                    let raw = format!("{bullet}{tag}{suffix}");
                    let expected = format!("{bullet}{expected}{suffix}");
                    let got = rename_refs(&raw, from, "New", is_org);
                    if got != expected {
                        failures.push(format!(
                            "org={is_org} {from:?}: {raw:?} => {got:?}, expected {expected:?}"
                        ));
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn audit_l3_02_rename_controls_and_literals() {
        assert_eq!(
            rename_refs("#+TITLE: Example\n* see #+TITLE\n", "+TITLE", "New", true),
            "#+TITLE: Example\n* see #New\n"
        );
        for is_org in [false, true] {
            // mldoc 1.5.9 (lsdoc harness) and Tine's renderer read ONE tag
            // named ` opener then #[[foo`, which does not reference `foo`; OG's
            // `rename-update-refs!` only rewrites the renamed page's referrers.
            assert_eq!(
                rename_in_block("a tag #[[ opener then #[[foo]] too", "foo", "New", is_org),
                "a tag #[[ opener then #[[foo]] too"
            );
            let raw = "[[foo]] #foo #[[foo]] #food https://ex.com/#foo";
            assert_eq!(
                rename_in_block(raw, "foo", "New", is_org),
                "[[New]] #New #New #food https://ex.com/#foo"
            );
            let (raw, expected) = if is_org {
                (
                    "* =#foo. #foo+bar #c++=\n#+BEGIN_SRC\n#foo. #foo+bar #c++\n#+END_SRC\n* #foo\n",
                    "* =#foo. #foo+bar #c++=\n#+BEGIN_SRC\n#foo. #foo+bar #c++\n#+END_SRC\n* #New\n",
                )
            } else {
                (
                    "- `#foo. #foo+bar #c++`\n```\n#foo. #foo+bar #c++\n```\n- #foo\n",
                    "- `#foo. #foo+bar #c++`\n```\n#foo. #foo+bar #c++\n```\n- #New\n",
                )
            };
            assert_eq!(rename_refs(raw, "foo", "New", is_org), expected);
            assert_eq!(rename_refs(raw, "c", "New", is_org), raw);
            assert_eq!(rename_refs(raw, "c++", "New", is_org), raw);
        }
    }

    #[test]
    fn audit_l3_02_bare_tags_property_matches_whole_members() {
        let raw = "tags:: foo+bar, c++, R&D, foo🎉, foo.bar, foo\n";
        assert_eq!(rename_tags_property(raw, "c", "New", false), raw);
        assert_eq!(rename_tags_property(raw, "R", "New", false), raw);
        assert_eq!(
            rename_tags_property(raw, "foo", "New", false),
            "tags:: foo+bar, c++, R&D, foo🎉, foo.bar, New\n"
        );
        assert_eq!(
            rename_tags_property(raw, "c++", "New", false),
            "tags:: foo+bar, New, R&D, foo🎉, foo.bar, foo\n"
        );
        assert_eq!(
            rename_rewrite(
                "tags:: #foo+bar, #foo., c++\n",
                false,
                &[("foo".into(), "New".into()), ("c++".into(), "Cpp".into())],
                FileNameFormat::TripleLowbar,
            ),
            "tags:: #foo+bar, #New., Cpp\n"
        );
    }

    #[test]
    fn rename_literal_runs_cross_code_boundaries_and_preserve_unicode() {
        let literal = "é猫 ordinary prose ".repeat(100);
        let raw = format!("{literal}`literal code` [[Old]]\n```\n{literal}[[Old]]\n```\n{literal}#Old [broken [ [[Old]]");
        let expected = format!("{literal}`literal code` [[New]]\n```\n{literal}[[Old]]\n```\n{literal}#New [broken [ [[New]]");
        assert_eq!(rename_in_block(&raw, "Old", "New", false), expected);
        let org = format!("{literal}\n#+BEGIN_SRC\n[[Old]]\n#+END_SRC\n{literal}[[Old]]");
        assert_eq!(
            rename_in_block(&org, "Old", "New", true),
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
            rename_in_block(
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
            rename_in_block("see [[Old]] and `[[Old]]`", "Old", "New", false),
            "see [[New]] and `[[Old]]`"
        );
        // fenced code is preserved verbatim; refs outside are renamed
        let raw = "before [[Old]]\n```js\nconst x = \"[[Old]]\"; // #Old\n```\nafter #Old";
        let got = rename_in_block(raw, "Old", "New", false);
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
        let out = rename_in_block(raw, "Old", "New", true);
        assert_eq!(
            out,
            "see [[New]] here\n#+BEGIN_SRC clojure\n(def s \"[[Old]]\") ; #Old\n#+END_SRC\nand [[New]] again\n"
        );
        // mldoc 1.5.9 emits Src for this input in Markdown too. The same
        // parser-owned literal protection applies in both formats.
        let md = rename_in_block(raw, "Old", "New", false);
        assert_eq!(md, out);
    }

    #[test]
    fn rename_rewrites_org_file_links() {
        // L1: org `[[file:…/<stem>.org][desc]]` / `[[file:…]]` targets the renamed
        // file's stem — rewrite it (org only), preserving dir, extension, and desc.
        let raw = "[[file:./pages/Old.org][The Old]] and [[file:./pages/Old.org]] and [[Old]]\n";
        let out = rename_in_block(raw, "Old", "New", true);
        assert_eq!(
            out,
            "[[file:./pages/New.org][The Old]] and [[file:./pages/New.org]] and [[New]]\n"
        );
        // Namespaced stem (`/`→`___`) and a non-matching file link are handled.
        assert_eq!(
            rename_in_block("[[file:./pages/a___Old.org][x]]", "a/Old", "New/Sub", true),
            "[[file:./pages/New___Sub.org][x]]"
        );
        assert_eq!(
            rename_in_block("[[file:./pages/Keep.org][k]]", "Old", "New", true),
            "[[file:./pages/Keep.org][k]]"
        );
        // Markdown (is_org=false) leaves file links alone (no org file-page links).
        assert_eq!(
            rename_in_block("[[file:./pages/Old.org]]", "Old", "New", false),
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
            assert_eq!(
                rename_block_raw(&raw, true, &renames, None, format, None),
                expected
            );
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
                                                  // No compatibility folding: fullwidth letters name another page
                                                  // (moved from tine-search `identity_fold`, deleted as a second key).
        assert_ne!(page_key("Ｔｉｎｅ"), page_key("Tine"));
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
            rename_in_block(raw, "Old", "New", false),
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
        // bracketed / #-prefixed values are references too, each rewritten once
        assert_eq!(
            rename_tags_property("tags:: [[Old]], #Old", "Old", "New", false),
            "tags:: [[New]], #New"
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
#[cfg(test)]
mod rename_site_tests {
    //! Conceptual model step 2, batch 6: the rename rewrites what the one
    //! reference answer says names the page, plus query page-name arguments.
    use super::*;

    fn rename(text: &str, org: bool, from: &str, to: &str) -> String {
        rename_rewrite(
            text,
            org,
            &[(from.to_owned(), to.to_owned())],
            FileNameFormat::TripleLowbar,
        )
    }

    /// REG-CM-RENAME-FENCE-ON-BULLET-LINE: a code fence that opens on the
    /// bullet's own line is code, so a rename leaves its body alone.
    #[test]
    fn a_fence_opening_on_the_bullet_line_stays_literal() {
        let text = "- ```\n  [[Old]] #Old\n  ```\n- [[Old]]\n";
        assert_eq!(
            rename(text, false, "Old", "New"),
            "- ```\n  [[Old]] #Old\n  ```\n- [[New]]\n"
        );
    }

    /// REG-CM-RENAME-URL-NESTED-ARRAY: nested arrays in a URL's query string
    /// are part of the URL, not links, so no page they seem to spell follows a
    /// rename.
    #[test]
    fn brackets_inside_a_url_are_no_link() {
        let text = "- see https://ex.com/q?[[[Old],[2]],[[Old],[3]]]\n";
        assert_eq!(rename(text, false, "[Old],[2", "New"), text);
        assert_eq!(rename(text, false, "Old", "New"), text);
    }

    /// Q4 (manager ruling 2026-11-03): Org labelled links, headline tags and
    /// drawer `:tags:` are references and follow.
    #[test]
    fn org_labelled_links_headline_tags_and_drawer_tags_follow() {
        let text = "* see [[Old][a label]] :Old:other:\n:PROPERTIES:\n:tags: Old, keep\n:END:\n";
        assert_eq!(
            rename(text, true, "Old", "New"),
            "* see [[New][a label]] :New:other:\n:PROPERTIES:\n:tags: New, keep\n:END:\n"
        );
        // A name a headline tag cannot hold keeps the tag; the link follows.
        assert_eq!(
            rename(text, true, "Old", "New page"),
            "* see [[New page][a label]] :Old:other:\n:PROPERTIES:\n:tags: New page, keep\n:END:\n"
        );
    }

    /// Q3: `word#Old` is a tag to lsdoc, mldoc and the evidence, so it follows
    /// (OG's `replace-tag-ref!` regex refuses it and leaves it stale).
    #[test]
    fn an_in_word_tag_follows() {
        assert_eq!(
            rename("- word#Old and #Old\n", false, "Old", "New"),
            "- word#New and #New\n"
        );
    }

    /// Q1: a property key that names the page is rewritten as OG's
    /// `replace-property-ref!` does (lower-cased, hyphenated); a bare alias
    /// member follows too (D-20261105-CM13, deliberate OG difference).
    #[test]
    fn property_keys_and_alias_members_follow() {
        assert_eq!(
            rename(
                "- block\n  Old:: value\n  alias:: Old, [[Old]]\n",
                false,
                "Old",
                "New Page"
            ),
            "- block\n  new-page:: value\n  alias:: New Page, [[New Page]]\n"
        );
        assert_eq!(
            rename(
                "* block\n:PROPERTIES:\n:Old: value\n:END:\n",
                true,
                "Old",
                "New"
            ),
            "* block\n:PROPERTIES:\n:new: value\n:END:\n"
        );
    }

    /// CM13: preserve separators and unrelated values, use the existing
    /// linkable-property spelling when a new member contains a comma.
    #[test]
    fn bare_alias_values_follow_in_markdown_and_org() {
        for (org, text, expected) in [
            (
                false,
                "aliases::  oLD ,keep, Old/sub\n\n- body\n",
                "aliases::  New ,keep, New/sub\n\n- body\n",
            ),
            (
                true,
                "#+alias:  oLD ,keep, Old/sub\n\n* body\n",
                "#+alias:  New ,keep, New/sub\n\n* body\n",
            ),
            (
                false,
                "alias:: Old, keep\n",
                "alias:: [[New, Page]], keep\n",
            ),
        ] {
            let new = if expected.contains("New, Page") {
                "New, Page"
            } else {
                "New"
            };
            assert_eq!(rename(text, org, "Old", new), expected);
        }
        assert_eq!(
            rename(
                "alias:: O\u{308}ld/sub\r\n\r\n- [[Öld/sub]] [[./Öld/sub]]\r\n",
                false,
                "Öld",
                "New"
            ),
            "alias:: New/sub\r\n\r\n- [[New/sub]] [[./Öld/sub]]\r\n"
        );
    }

    /// Page properties (the pre-block) follow, mapped back to their own bytes.
    #[test]
    fn page_properties_follow_in_place() {
        assert_eq!(
            rename(
                "tags::  Old ,keep\nOld:: v\n\n- [[Old]]\n",
                false,
                "Old",
                "New"
            ),
            "tags::  New ,keep\nnew:: v\n\n- [[New]]\n"
        );
    }

    /// Q2 (declared OG difference): a `[[Old]]` the parser does not read as a
    /// reference stays, where OG's literal replace would rewrite it.
    #[test]
    fn non_references_stay() {
        for text in [
            "- $[[Old]]$ and <!-- [[Old]] --> and \\[[Old]]\n",
            "- task\n  :LOGBOOK:\n  CLOCK: [[Old]]\n  :END:\n",
            "- `[[Old]]`\n",
        ] {
            assert_eq!(rename(text, false, "Old", "New"), text, "{text:?}");
        }
    }

    /// D-20261031-CMQ: a query's page-name arguments follow like links, on
    /// every page; its search text stays.
    #[test]
    fn query_page_names_follow_and_search_text_stays() {
        let text = "- {{query (and [[Old]] (page \"Old\") (page-ref Old) #Old \"Old\")}}\n";
        assert_eq!(
            rename(text, false, "Old", "New name"),
            "- {{query (and [[New name]] (page \"New name\") (page-ref \"New name\") #[[New name]] \"Old\")}}\n"
        );
        let tql = "- {{tine-query @page name = 'Old' and content = 'Old'}}\n";
        assert_eq!(
            rename(tql, false, "Old", "New"),
            "- {{tine-query @page name = 'New' and content = 'Old'}}\n"
        );
        // A query macro inside code is no macro.
        let code = "- ```\n  {{query [[Old]]}}\n  ```\n";
        assert_eq!(rename(code, false, "Old", "New"), code);
    }

    /// REG-QS-RENAME-CODE-IN-DEFINITION: code is literal text, so a
    /// `{{tine-query}}` in an inline code span or a fence is no query macro
    /// and a rename leaves it, in a block's text and in a property's value
    /// (the parser keeps a property line as one node; the app shows each value
    /// as inline text), while the real macro beside it follows.
    #[test]
    fn a_tine_query_in_code_inside_a_definition_stays() {
        let def = "{{tine-query @page and name = 'Old'}}";
        let to = "{{tine-query @page and name = 'New'}}";
        let og = "{{query (page \"Old\")}}";
        let og_to = "{{query (page \"New\")}}";
        for (text, want) in [
            // In the block's text.
            (
                format!("- {def}\n  tine.export:: out\n  `{def}`\n"),
                format!("- {to}\n  tine.export:: out\n  `{def}`\n"),
            ),
            (
                format!("- {def}\n  tine.export:: out\n  ```\n  {def}\n  ```\n"),
                format!("- {to}\n  tine.export:: out\n  ```\n  {def}\n  ```\n"),
            ),
            (
                format!("- x\n  tine.export:: out\n  ~~~\n  {def}\n  ~~~\n  {def}\n"),
                format!("- x\n  tine.export:: out\n  ~~~\n  {def}\n  ~~~\n  {to}\n"),
            ),
            (
                format!("- x\n  tine.export:: out\n  ``{def}`` {def}\n"),
                format!("- x\n  tine.export:: out\n  ``{def}`` {to}\n"),
            ),
            (
                format!("- `{def}` and {def}\n  tine.share:: a/b\n"),
                format!("- `{def}` and {to}\n  tine.share:: a/b\n"),
            ),
            // In a block property's value, a macro outside code still follows.
            (
                format!("- x\n  q:: `{def}`\n"),
                format!("- x\n  q:: `{def}`\n"),
            ),
            (
                format!("- {def}\n  tine.export:: `{def}`\n"),
                format!("- {to}\n  tine.export:: `{def}`\n"),
            ),
            (
                format!("- x\n  q:: `{og}` {og}\n"),
                format!("- x\n  q:: `{og}` {og_to}\n"),
            ),
            (format!("- x\n  q:: {def}\n"), format!("- x\n  q:: {to}\n")),
            // In a page property's value.
            (
                format!("tags:: `{def}`\n\n- x\n"),
                format!("tags:: `{def}`\n\n- x\n"),
            ),
            (
                format!("type:: `{def}` and {def}\n\n- x\n"),
                format!("type:: `{def}` and {to}\n\n- x\n"),
            ),
            (
                format!("type:: {def}\n\n- x\n"),
                format!("type:: {to}\n\n- x\n"),
            ),
        ] {
            assert_eq!(rename(&text, false, "Old", "New"), want, "{text:?}");
        }
        let org = format!(
            "* {def}\n:PROPERTIES:\n:tine.export: out\n:Q: ~{def}~ {def}\n:END:\n#+BEGIN_SRC\n{def}\n#+END_SRC\n=a {def}=\n~{def}~\n"
        );
        let org_want = org.replacen("'Old'", "'New'", 1).replacen(
            &format!("~{def}~ {def}"),
            &format!("~{def}~ {to}"),
            1,
        );
        assert_eq!(rename(&org, true, "Old", "New"), org_want, "{org:?}");
    }

    /// Item 2 of qs-leftovers-1109: a double-quoted TQL operand is an
    /// identifier, not a name (the query is invalid and shares nothing), so its
    /// bytes stay; the single-quoted spelling in the next macro follows, and the
    /// OG DSL's quoted `(page "Old")` follows in its own spelling.
    #[test]
    fn a_double_quoted_tql_operand_stays_and_the_readable_spellings_follow() {
        for (text, want) in [
            (
                "- {{tine-query @page and name = \"Old\"}}\n",
                "- {{tine-query @page and name = \"Old\"}}\n",
            ),
            (
                "- {{tine-query @page and name = 'Old'}} {{tine-query @page and name = \"Old\"}}\n",
                "- {{tine-query @page and name = 'New'}} {{tine-query @page and name = \"Old\"}}\n",
            ),
            (
                "- {{query (and (page \"Old\") (page-ref Old))}}\n",
                "- {{query (and (page \"New\") (page-ref New))}}\n",
            ),
        ] {
            assert_eq!(rename(text, false, "Old", "New"), want, "{text:?}");
        }
    }

    /// CRLF pages keep their line ends; only the names change.
    #[test]
    fn a_crlf_page_is_rewritten_in_place() {
        assert_eq!(
            rename("- a\r\n  - [[Old]] #Old\r\n", false, "Old", "New"),
            "- a\r\n  - [[New]] #New\r\n"
        );
    }

    /// Markers and priorities name a page but never follow (D-20261019-M2(4)).
    #[test]
    fn markers_and_priorities_stay() {
        assert_eq!(
            rename("- TODO [#A] [[TODO]]\n", false, "TODO", "Task"),
            "- TODO [#A] [[Task]]\n"
        );
        assert_eq!(
            rename("- TODO [#A] x\n", false, "A", "B"),
            "- TODO [#A] x\n"
        );
    }
}

#[cfg(test)]
mod tql_near_seed_rename_tests {
    /// docs/contracts/tql-links-near.md "Renames": a bracketed seed inside a
    /// `near(...)` follows a rename through the ordinary reference rewrite.
    #[test]
    fn a_rename_rewrites_bracketed_near_seeds() {
        let raw = "- {{tine-query @page and near(name in ([[Project]], [[Notes]]), 2)}}\n";
        let renames: std::collections::HashMap<String, String> =
            [("project".to_string(), "Project archive".to_string())]
                .into_iter()
                .collect();
        assert_eq!(
            super::rename_rewrite_keyed(
                raw,
                false,
                &renames,
                crate::config::FileNameFormat::TripleLowbar
            ),
            "- {{tine-query @page and near(name in ([[Project archive]], [[Notes]]), 2)}}\n"
        );
    }
}
