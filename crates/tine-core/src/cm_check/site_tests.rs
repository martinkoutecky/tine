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

/// A disagreement the check surfaces (receipt, Continuation question Q-f): the
/// evidence names a Markdown file link `[Old](file:x.md)` by its label, so it
/// is a link to `Old`, but the writer respells a Markdown label link only when
/// its target is `[[…]]`, so it keeps the label and the check refuses its
/// output. Org file links are respelled by their label (above). A writer that
/// respells the label flips the first assertion.
#[test]
fn a_markdown_file_link_label_is_a_link_the_writer_keeps() {
    for before in ["- [Old](file:x.md)\n", "- [Old](../pages/x.md)\n"] {
        assert_eq!(written("Old", "New", before, MD), before);
        assert!(!accepts("Old", "New", before, before, MD), "{before:?}");
        let after = before.replace("[Old]", "[New]");
        assert!(accepts("Old", "New", before, &after, MD), "{before:?}");
    }
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
