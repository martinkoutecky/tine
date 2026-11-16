//! The site kinds a rename respells beyond references (query page-name
//! arguments, links in other macros' arguments, Org `file:` link stems): each
//! is observed, the rename writer's own output for it passes the check, and a
//! wrong output fails it, in Markdown and in Org. The first test pins that
//! every rule the writer rewrites has an observed kind.

use super::*;
use crate::query::macro_text::FormFamily;
use crate::reference_evidence::RewriteTarget;
use std::collections::HashMap;

const NAMES: FileNameFormat = FileNameFormat::TripleLowbar;
const MD: Format = Format::Md;
const ORG: Format = Format::Org;

/// O1/O7: exercise the spelling decisions shared with the writer across
/// every non-nested site family, with inline syntax and separator names.
#[test]
fn spelling_rules_agree_at_every_site_kind() {
    let mut failures = Vec::new();
    for (before, format) in [
        ("- #Old\n", MD),
        ("- #[[Old]]\n", MD),
        ("- block\n  tags:: Old\n", MD),
        ("alias:: Old\n\n- body\n", MD),
        ("- old:: value\n", MD),
        ("* heading :Old:\n", ORG),
        ("- {{renderer [[Old]]}}\n", MD),
        ("- {{query (page Old)}}\n", MD),
        ("- {{tine-query @page and name = 'Old'}}\n", MD),
        ("- {{query #Old}}\n", MD),
        ("#+alias: Old\n* body\n", ORG),
        ("alias:: [Old](file:x.md)\n\n- body\n", MD),
        ("#+alias: [[file:x.org][Old]]\n* body\n", ORG),
        ("* h\n:PROPERTIES:\n:tags: Old\n:END:\n", ORG),
        ("* [[Old]]\n", ORG),
        ("* [[file:Old.org][label]]\n", ORG),
        ("- [[Old]]\n", MD),
        ("* [[Old][label]]\n", ORG),
        ("- [label]([[Old]])\n", MD),
        ("- **[[Old]]**\n", MD),
        ("- {{embed [[Old]]}}\n", MD),
    ] {
        for to in [
            "a]b",
            r"New\\Tail",
            "a*b",
            "a#b",
            "a_b",
            "a`b",
            "a,b",
            "a:b",
            "a\"b",
            "a b",
            "a+b",
        ] {
            let after = written("Old", to, before, format);
            if !accepts("Old", to, before, &after, format) {
                failures.push(format!(
                    "{before:?} -> {to:?}: {after:?}; reading {:?}",
                    read(&after, format)
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Sharing the spelling answer must not erase the parser's actual name.
/// Equal native text with a different observed name is still a P2 mismatch.
#[test]
fn a_labels_parsed_name_is_compared_even_with_equal_native_text() {
    let before = read("- [Old](file:x.md)\n", MD);
    let rename = Rename::new("Old", "New", None, NAMES);
    let want = expected(
        &rename,
        MD,
        Expect::File {
            content: &before,
            skip: false,
        },
    );
    let mut actual = read("- [New](file:x.md)\n", MD);
    assert!(matches(&want, &actual, false));
    assert!(!matches(
        &want,
        &read("- [**New**](file:x.md)\n", MD),
        false
    ));
    for atom in &mut actual.blocks[0].body {
        if let Atom::Site(s) = atom {
            s.name = "Another".into();
        }
    }
    assert!(!matches(&want, &actual, false));
}

/// A label is still the property member its reader reports, not an
/// ordinary body link. Alias allocation depends on this classification.
#[test]
fn file_link_labels_remain_alias_members() {
    for (before, format) in [
        ("alias:: [Old](file:x.md)\n\n- body\n", MD),
        ("#+alias: [[file:x.org][Old]]\n* body\n", ORG),
    ] {
        let content = read(before, format);
        assert_eq!(content.header[0].role, HeaderRole::Aliases);
        assert!(text_sites(&content.header[0].text)
            .any(|s| s.kind == SiteKind::AliasName && s.name == "Old"));
        let after = written("Old", "New", before, format);
        assert!(accepts("Old", "New", before, &after, format));
    }
}

fn read(text: &str, format: Format) -> Content {
    observe(text, format, NAMES)
}

fn kinds(text: &str, format: Format) -> Vec<SiteKind> {
    let c = read(text, format);
    c.header
        .iter()
        .flat_map(|h| text_sites(&h.text))
        .chain(c.blocks.iter().flat_map(|b| text_sites(&b.body)))
        .map(|s| s.kind)
        .collect()
}

fn accepts(old: &str, new: &str, before: &str, after: &str, format: Format) -> bool {
    let rename = Rename::new(old, new, None, NAMES);
    let content = read(before, format);
    let want = expected(
        &rename,
        format,
        Expect::File {
            content: &content,
            skip: false,
        },
    );
    matches(&want, &read(after, format), false)
}

/// The rename writer's text for `before` (`refs::rename_proposal`).
fn written(old: &str, new: &str, before: &str, format: Format) -> String {
    let map = HashMap::from([(crate::refs::page_key(old), new.to_owned())]);
    crate::refs::rename_proposal(before, format == ORG, &map, &HashMap::new(), NAMES)
        .unwrap()
        .text
}

/// The writer turns `before` into `after`, the check accepts that, and it
/// rejects `wrong`.
fn case(old: &str, new: &str, before: &str, after: &str, wrong: &str, format: Format) {
    assert_eq!(
        written(old, new, before, format),
        after,
        "writer on {before:?}"
    );
    assert!(
        accepts(old, new, before, after, format),
        "{before:?} → {after:?} refused"
    );
    assert!(
        !accepts(old, new, before, wrong, format),
        "{before:?} → {wrong:?} accepted"
    );
}

fn first_raw(text: &str, format: Format) -> String {
    let doc = if format == ORG {
        crate::org::parse_org(text)
    } else {
        crate::doc::parse(text)
    };
    doc.roots[0].raw().to_owned()
}

/// Every reference rule (the closed set `reference_rule::RULES`), every
/// rewrite target and every query family the writer rewrites has a site kind
/// here. A new rule fails the table; a new target or family fails to compile
/// (the matches are exhaustive).
#[test]
fn every_rewriter_rule_has_an_observed_kind() {
    use crate::reference_evidence::reference_rule::RULES;
    let rules: &[(&str, &str, SiteKind)] = &[
        ("explicit_link", "- [[Old]]\n", SiteKind::Link),
        ("explicit_nested_link", "- [[a [[Old]]]]\n", SiteKind::Link),
        ("explicit_tag", "- #Old\n", SiteKind::Tag),
        ("explicit_embed", "- {{embed [[Old]]}}\n", SiteKind::Embed),
        (
            "explicit_property_key",
            "- old:: v\n",
            SiteKind::PropertyKey,
        ),
        ("explicit_task_marker", "- TODO x\n", SiteKind::Task),
        ("explicit_priority", "- [#A] x\n", SiteKind::Priority),
        (
            "implicit_linkable_property",
            "- tags:: Old\n",
            SiteKind::PropertyValue,
        ),
    ];
    let mut named: Vec<&str> = rules.iter().map(|r| r.0).collect();
    let mut all = RULES.to_vec();
    named.sort_unstable();
    all.sort_unstable();
    assert_eq!(named, all, "a reference rule without an observed kind");
    for (rule, text, kind) in rules {
        let doc = crate::doc::parse(text);
        let evidence = doc.roots[0].projection();
        assert!(
            evidence
                .reference_source()
                .explicit
                .iter()
                .any(|r| r.rule == *rule),
            "{text:?} has no {rule}"
        );
        assert!(kinds(text, MD).contains(kind), "{rule}: {text:?}");
    }
    let target_kind = |t: &RewriteTarget| match t {
        RewriteTarget::FileLink(_) => SiteKind::OrgFile,
        RewriteTarget::MacroLink(_) => SiteKind::MacroArg,
    };
    for (text, format) in [
        ("* [[file:Old.org][x]]\n", ORG),
        ("- {{cloze [[Old]]}}\n", MD),
        ("- {{cloze [[a [[Old]]]]}}\n", MD),
    ] {
        let targets =
            crate::reference_evidence::rename_targets(&first_raw(text, format), format == ORG);
        assert!(!targets.is_empty(), "{text:?}");
        for t in &targets {
            assert!(kinds(text, format).contains(&target_kind(t)), "{text:?}");
        }
    }
    let family = |f: FormFamily| match f {
        FormFamily::Edn => "- {{query [[Old]]}}\n",
        FormFamily::Tql => "- {{tine-query @page and name = 'Old'}}\n",
    };
    for f in [FormFamily::Edn, FormFamily::Tql] {
        assert!(kinds(family(f), MD).contains(&SiteKind::Query), "{f:?}");
    }
    // Every name the evidence reports is a site, at every nesting level: an
    // enclosing name and its inner names, a link inside a tag, the levels of
    // a link in a macro argument.
    for (text, format) in [
        ("- [[a [[Old]]]]\n", MD),
        ("- [[a [[b [[Old]]]]]]\n", MD),
        ("- #[[a [[Old]]]]\n", MD),
        ("- #[[Old]]\n", MD),
        ("- {{cloze [[a [[Old]]]]}}\n", MD),
        ("* x [[a [[Old]]]]\n", ORG),
    ] {
        let raw = first_raw(text, format);
        let doc = if format == ORG {
            crate::org::parse_org(text)
        } else {
            crate::doc::parse(text)
        };
        let projection = doc.roots[0].projection();
        let mut reported: Vec<String> = projection
            .reference_source()
            .explicit
            .iter()
            .map(|r| r.name.clone())
            .chain(
                crate::reference_evidence::rename_targets(&raw, format == ORG)
                    .into_iter()
                    .filter_map(|t| match t {
                        RewriteTarget::MacroLink(p) => Some(p.name),
                        RewriteTarget::FileLink(_) => None,
                    }),
            )
            .collect();
        let c = read(text, format);
        let mut observed: Vec<String> = Vec::new();
        let mut stack: Vec<&Site> = c.blocks.iter().flat_map(|b| text_sites(&b.body)).collect();
        while let Some(s) = stack.pop() {
            observed.push(s.name.clone());
            stack.extend(s.inner.iter().map(|(_, i)| i));
        }
        for list in [&mut reported, &mut observed] {
            list.sort_unstable();
            list.dedup();
        }
        assert_eq!(observed, reported, "{text:?}");
    }
}

/// The legality of a macro's respelled tokens is one reading of the macro
/// (design "Cost"): a macro with 1, 10 or 100 renamed tokens is read once.
/// Red before: once per token (1, 10, 100).
#[test]
fn a_query_macro_is_read_once_however_many_tokens_it_holds() {
    for n in [1, 10, 100] {
        let before = format!("- {{{{query (or {})}}}}\n", "[[Old]] ".repeat(n).trim_end());
        let after = before.replace("Old", "New");
        MACRO_READINGS.with(|r| r.set(0));
        assert!(accepts("Old", "New", &before, &after, MD));
        assert_eq!(MACRO_READINGS.with(|r| r.get()), 1, "{n} tokens");
    }
}

/// A macro's tokens follow together or keep their bytes, the writer's rule: a
/// new name that would end the macro leaves every token of it, and the check
/// accepts exactly that.
#[test]
fn a_query_macro_that_cannot_hold_the_name_keeps_every_token() {
    let before = "- {{query (or [[Old]] [[Old]])}}\n";
    assert_eq!(written("Old", "a}b", before, MD), before);
    assert!(accepts("Old", "a}b", before, before, MD));
    assert!(!accepts(
        "Old",
        "a}b",
        before,
        &before.replace("Old", "a}b"),
        MD
    ));
}

#[test]
fn a_query_page_argument_is_respelled() {
    case(
        "Old",
        "New",
        "- {{query [[Old]]}}\n",
        "- {{query [[New]]}}\n",
        "- {{query [[Old]]}}\n",
        MD,
    );
    case(
        "Old",
        "New Name",
        "- {{query (and [[Old]] (page Old))}}\n",
        "- {{query (and [[New Name]] (page \"New Name\"))}}\n",
        "- {{query (and [[New Name]] (page Old))}}\n",
        MD,
    );
    case(
        "Old",
        "New",
        "- {{tine-query @page and name = 'Old'}}\n",
        "- {{tine-query @page and name = 'New'}}\n",
        "- {{tine-query @page and name = 'Old'}}\n",
        MD,
    );
    case(
        "Old",
        "New",
        "q:: {{query [[Old]]}}\n- x {{query #Old}}\n",
        "q:: {{query [[New]]}}\n- x {{query #New}}\n",
        "q:: {{query [[Old]]}}\n- x {{query #New}}\n",
        MD,
    );
    case(
        "Old",
        "New",
        "* {{query [[Old]]}}\n",
        "* {{query [[New]]}}\n",
        "* {{query [[Old]]}}\n",
        ORG,
    );
    case(
        "Old",
        "New",
        "* h\n:PROPERTIES:\n:q: {{query (page Old)}}\n:END:\n",
        "* h\n:PROPERTIES:\n:q: {{query (page New)}}\n:END:\n",
        "* h\n:PROPERTIES:\n:q: {{query (page Old)}}\n:END:\n",
        ORG,
    );
}

/// Negative controls of the kind: search text, a macro in code, and a name
/// the macro cannot hold are no site the rename respells.
#[test]
fn query_search_text_code_and_unspellable_names_stay() {
    for (before, wrong, format) in [
        ("- {{query \"Old\"}}\n", "- {{query \"New\"}}\n", MD),
        ("- `{{query [[Old]]}}`\n", "- `{{query [[New]]}}`\n", MD),
        (
            "- q:: `{{query [[Old]]}}`\n",
            "- q:: `{{query [[New]]}}`\n",
            MD,
        ),
        ("* {{query \"Old\"}}\n", "* {{query \"New\"}}\n", ORG),
        (
            "* =x {{query [[Old]]}}=\n",
            "* =x {{query [[New]]}}=\n",
            ORG,
        ),
    ] {
        case("Old", "New", before, before, wrong, format);
    }
    let before = "- {{query [[Old]]}}\n";
    case("Old", "a}b", before, before, "- {{query [[a}b]]}}\n", MD);
    let before = "* {{query [[Old]]}}\n";
    case("Old", "a}b", before, before, "* {{query [[a}b]]}}\n", ORG);
}

#[test]
fn a_link_in_a_macro_argument_is_respelled() {
    case(
        "Old",
        "New",
        "- {{cloze [[Old]]}} x\n",
        "- {{cloze [[New]]}} x\n",
        "- {{cloze [[Old]]}} x\n",
        MD,
    );
    case(
        "Old",
        "New",
        "- {{cloze [[a [[Old]]]]}}\n",
        "- {{cloze [[a [[New]]]]}}\n",
        "- {{cloze [[a [[Old]]]]}}\n",
        MD,
    );
    case(
        "Old",
        "New",
        "* {{cloze [[Old]]}}\n",
        "* {{cloze [[New]]}}\n",
        "* {{cloze [[Old]]}}\n",
        ORG,
    );
}

/// Negative controls of the kind: plain macro text, and a name with `}`.
#[test]
fn macro_text_and_a_closing_brace_stay() {
    for (before, wrong, format) in [
        ("- {{cloze Old}}\n", "- {{cloze New}}\n", MD),
        ("* {{cloze Old}}\n", "* {{cloze New}}\n", ORG),
    ] {
        case("Old", "New", before, before, wrong, format);
    }
    let before = "- {{cloze [[Old]]}}\n";
    case("Old", "a}b", before, before, "- {{cloze [[a}b]]}}\n", MD);
    let before = "* {{cloze [[Old]]}}\n";
    case("Old", "a}b", before, before, "* {{cloze [[a}b]]}}\n", ORG);
}

#[test]
fn an_org_file_link_stem_is_respelled() {
    case(
        "Old",
        "New",
        "* see [[file:Old.org][there]]\n",
        "* see [[file:New.org][there]]\n",
        "* see [[file:Old.org][there]]\n",
        ORG,
    );
    case(
        "Old",
        "New",
        "* [[file:Old.org]]\n",
        "* [[file:New.org]]\n",
        "* [[file:Old.org]]\n",
        ORG,
    );
    case(
        "Old",
        "New",
        "* [[file:Old.org][Old]]\n",
        "* [[file:New.org][New]]\n",
        "* [[file:New.org][Old]]\n",
        ORG,
    );
    case(
        "Old",
        "New",
        "* [[file:../pages/Old___x.org][x]]\n",
        "* [[file:../pages/New___x.org][x]]\n",
        "* [[file:../pages/New/x.org][x]]\n",
        ORG,
    );
}

/// The evidence names a Markdown file link `[Old](file:x.md)` by its label,
/// so it is a link to `Old` (Q-f), and slice 2's label rule respells a
/// contiguous name in the label with its formatting kept: the writer's output
/// passes, the unchanged text fails. Org file links are respelled by their
/// label too (above).
#[test]
fn a_markdown_file_link_label_is_respelled_with_its_formatting() {
    for (before, after) in [
        ("- [Old](file:x.md)\n", "- [New](file:x.md)\n"),
        ("- [Old](../pages/x.md)\n", "- [New](../pages/x.md)\n"),
        ("- [**Old**](file:x.md)\n", "- [**New**](file:x.md)\n"),
    ] {
        case("Old", "New", before, after, before, MD);
    }
}

/// A name split by markup in a Markdown file-link label (`O**ld**`) has no
/// contiguous spelling to replace: the writer leaves it and reports it (O7),
/// and the check accepts the text it left.
#[test]
fn a_markdown_file_link_label_split_by_markup_is_left() {
    let before = "- [O**ld**](file:x.md)\n";
    assert_eq!(written("Old", "New", before, MD), before);
    assert!(accepts("Old", "New", before, before, MD));
}

/// Review round 2, finding 1: every level of a nested reference is a site.
/// An inner name is respelled inside its enclosing name, and the enclosing
/// name (with its inner names respelled) follows the rename itself: the
/// writer's output passes, the unchanged text fails. The reviewer's two
/// inputs first (a nested link inside a tag; a renamed enclosing name).
#[test]
fn every_level_of_a_nested_reference_is_respelled() {
    let unchanged = |old, new, before, after, format| case(old, new, before, after, before, format);
    unchanged("Old", "New", "- #[[a [[Old]]]]\n", "- #[[a [[New]]]]\n", MD);
    unchanged("a [[X]]", "New", "- [[a [[X]]]]\n", "- [[New]]\n", MD);
    unchanged("Old", "New", "- [[a [[Old]]]]\n", "- [[a [[New]]]]\n", MD);
    unchanged(
        "Old",
        "New",
        "- [[ a  [[Old]] ]]\n",
        "- [[ a  [[New]] ]]\n",
        MD,
    );
    unchanged(
        "Old",
        "New",
        "- [[a [[b [[Old]]]]]]\n",
        "- [[a [[b [[New]]]]]]\n",
        MD,
    );
    unchanged(
        "b [[Old]]",
        "New",
        "- [[a [[b [[Old]]]]]]\n",
        "- [[a [[New]]]]\n",
        MD,
    );
    unchanged(
        "Old",
        "New",
        "- tags:: [[a [[Old]]]]\n",
        "- tags:: [[a [[New]]]]\n",
        MD,
    );
    unchanged(
        "Old",
        "New",
        "* x [[a [[Old]]]]\n",
        "* x [[a [[New]]]]\n",
        ORG,
    );
    unchanged(
        "Old",
        "New",
        "- {{cloze [[a [[Old]]]]}}\n",
        "- {{cloze [[a [[New]]]]}}\n",
        MD,
    );
    unchanged(
        "a [[Old]]",
        "New",
        "- {{cloze [[a [[Old]]]]}}\n",
        "- {{cloze [[New]]}}\n",
        MD,
    );
    // A link that spells its tag (`#[[Old]]`) is a site too.
    unchanged("Old", "New", "- #[[Old]]\n", "- #[[New]]\n", MD);
    // Respelling only one level of a nested name is refused.
    assert!(!accepts(
        "Old",
        "New",
        "- [[a [[Old]]]]\n",
        "- [[New]]\n",
        MD
    ));
    assert!(!accepts(
        "a [[X]]",
        "New",
        "- [[a [[X]]]]\n",
        "- [[a [[New]]]]\n",
        MD
    ));
}

/// A disagreement the check surfaces (review round 2): renaming `Old`, the
/// writer respells the enclosing namespace name `Old/[[Old]]` and drops the
/// inner edit, leaving a link to the renamed page; the model respells both
/// levels. A writer that respells both flips the first assertion.
#[test]
fn a_nested_namespace_name_respells_both_levels() {
    let before = "- [[Old/[[Old]]]]\n";
    assert_eq!(written("Old", "New", before, MD), "- [[New/[[Old]]]]\n");
    assert!(!accepts("Old", "New", before, "- [[New/[[Old]]]]\n", MD));
    assert!(accepts("Old", "New", before, "- [[New/[[New]]]]\n", MD));
}

/// What the rename plan does with a page whose own name holds the renamed
/// name (review round 2, finding 1): `a [[Old]]` is neither `Old` nor in its
/// namespace, so its file stays where it is.
#[test]
fn a_page_whose_name_holds_the_renamed_name_stays() {
    let rename = Rename::new("Old", "New", None, NAMES);
    let path = "pages/a [[Old]].md";
    assert_eq!(expected_path(&rename, path, "a [[Old]]", false), path);
}

/// Negative controls of the kind: a stem naming another page, and Markdown,
/// where a file link's path is no site.
#[test]
fn another_stem_and_a_markdown_file_link_stay() {
    for (before, wrong, format) in [
        ("* [[file:Other.org][x]]\n", "* [[file:New.org][x]]\n", ORG),
        ("- [x](file:Old.md)\n", "- [x](file:New.md)\n", MD),
        ("- [x](../pages/Old.md)\n", "- [x](../pages/New.md)\n", MD),
    ] {
        case("Old", "New", before, before, wrong, format);
    }
}

/// A tag in a property value respelled into its bracket form (`#x` →
/// `#[[Next Name]]`) is one site, as a block's tag is: the `[[Next Name]]`
/// that spells it is no site of its own. (Found by the transaction check on
/// `client::page_rename_matches_legacy_for_refs_namespace_case_and_tags`.)
#[test]
fn a_property_value_tag_in_bracket_form_is_one_site() {
    let c = read("tags:: #[[Next Name]]\n", MD);
    let sites: Vec<_> = c.header.iter().flat_map(|h| text_sites(&h.text)).collect();
    assert_eq!(sites.len(), 1, "{sites:?}");
    assert!(sites[0].inner.is_empty(), "{sites:?}");
    case(
        "x",
        "Next Name",
        "tags:: #x\n- a\n",
        "tags:: #[[Next Name]]\n- a\n",
        "tags:: #[[Other]]\n- a\n",
        MD,
    );
}
