//! `observe`, `expected` and `matches` end to end on small texts, one rule
//! each, with a wrong output per rule that the check must reject.

use super::*;
use crate::date::JournalFormat;

const NAMES: FileNameFormat = FileNameFormat::Legacy;

fn observe(text: &str, format: Format) -> Content {
    super::observe(text, format, NAMES)
}

fn md(text: &str) -> Content {
    observe(text, Format::Md)
}

/// The expected reading of a referrer `before` under `old` → `new`.
fn renamed(old: &str, new: &str, before: &str, format: Format) -> Semantic {
    let days = JournalFormat::new(Some("yyyy/MM/dd"), None);
    let rename = Rename::new(old, new, Some(&days), NAMES);
    let content = observe(before, format);
    expected(
        &rename,
        format,
        Expect::File {
            content: &content,
            skip: false,
        },
    )
}

fn rename_ok(old: &str, new: &str, before: &str, after: &str, format: Format) -> bool {
    matches(
        &renamed(old, new, before, format),
        &observe(after, format),
        false,
    )
}

/// The expected survivor of merging page `A` (`source`) into `B` (`target`).
fn merged(source: &str, target: &str, format: Format) -> Semantic {
    let rename = Rename::new("A", "B", None, NAMES);
    expected(
        &rename,
        format,
        Expect::Survivor {
            source: &observe(source, format),
            target: &observe(target, format),
        },
    )
}

fn merge_ok(source: &str, target: &str, after: &str) -> bool {
    matches(&merged(source, target, Format::Md), &md(after), true)
}

#[test]
fn a_link_is_respelled_and_a_lost_block_is_refused() {
    let before = "- see [[Old]] here\n- second\n";
    assert!(rename_ok(
        "Old",
        "New",
        before,
        "- see [[New]] here\n- second\n",
        Format::Md
    ));
    // Negative controls: the old spelling kept, a block lost.
    assert!(!rename_ok("Old", "New", before, before, Format::Md));
    assert!(!rename_ok(
        "Old",
        "New",
        before,
        "- see [[New]] here\n",
        Format::Md
    ));
}

#[test]
fn a_tag_takes_the_writers_tag_spelling() {
    for (new, after) in [("New", "- #New\n"), ("Two Words", "- #[[Two Words]]\n")] {
        assert!(
            rename_ok("Old", new, "- #Old\n", after, Format::Md),
            "{new}"
        );
    }
    assert!(rename_ok(
        "Old",
        "New",
        "- #[[Old]]\n",
        "- #[[New]]\n",
        Format::Md
    ));
    // A bare tag cannot hold a space: `#Two Words` is a different reading.
    assert!(!rename_ok(
        "Old",
        "Two Words",
        "- #Old\n",
        "- #Two Words\n",
        Format::Md
    ));
}

#[test]
fn a_property_value_and_a_property_key_are_respelled() {
    let before = "- a\n- tags:: Old, x\n  old:: v\n";
    assert!(rename_ok(
        "Old",
        "New Key",
        before,
        "- a\n- tags:: New Key, x\n  new-key:: v\n",
        Format::Md
    ));
    // The key keeps O7's key spelling, never the page name.
    assert!(!rename_ok(
        "Old",
        "New Key",
        before,
        "- a\n- tags:: New Key, x\n  New Key:: v\n",
        Format::Md
    ));
}

#[test]
fn a_nested_title_respells_its_prefix_and_its_sites() {
    let before = "title:: Old/sub [[Old]]\n- x\n";
    assert!(rename_ok(
        "Old",
        "New",
        before,
        "title:: New/sub [[New]]\n- x\n",
        Format::Md
    ));
    assert!(!rename_ok(
        "Old",
        "New",
        before,
        "title:: New/sub [[Old]]\n- x\n",
        Format::Md
    ));
}

#[test]
fn a_day_site_stays_and_a_respelled_day_is_refused() {
    let before = "- [[2026/10/26]] [[2026/x]]\n";
    let after = "- [[2026/10/26]] [[B/x]]\n";
    assert!(rename_ok("2026", "B", before, after, Format::Md));
    // Negative control: the day respelled like a namespace member.
    assert!(!rename_ok(
        "2026",
        "B",
        before,
        "- [[B/10/26]] [[B/x]]\n",
        Format::Md
    ));
}

#[test]
fn an_unspellable_site_stays() {
    // An Org headline tag cannot hold a space.
    assert!(rename_ok(
        "Old",
        "Two Words",
        "* h :Old:\n",
        "* h :Old:\n",
        Format::Org
    ));
    assert!(rename_ok(
        "Old",
        "New",
        "* h :Old:\n",
        "* h :New:\n",
        Format::Org
    ));
    assert!(!rename_ok(
        "Old",
        "Two Words",
        "* h :Old:\n",
        "* h :Two_Words:\n",
        Format::Org
    ));
    // No key spelling holds a `:`.
    let before = "- a\n- old:: v\n";
    assert!(rename_ok("Old", "a:b", before, before, Format::Md));
}

#[test]
fn tasks_priorities_embeds_and_nested_links() {
    let before = "- TODO [#A] {{embed [[Old]]}} [[a [[Old]] c]]\n";
    let after = "- TODO [#A] {{embed [[New]]}} [[a [[New]] c]]\n";
    assert!(rename_ok("Old", "New", before, after, Format::Md));
    // Negative control: the nested name kept.
    let kept = "- TODO [#A] {{embed [[New]]}} [[a [[Old]] c]]\n";
    assert!(!rename_ok("Old", "New", before, kept, Format::Md));
    // A task marker is never respelled, whatever the rename.
    assert!(rename_ok(
        "todo",
        "Done",
        "- TODO x\n",
        "- TODO x\n",
        Format::Md
    ));
}

#[test]
fn a_merge_unites_aliases_by_reading_identity() {
    let (source, target) = ("alias:: X, y\n- s\n", "alias:: Y\n- t\n");
    assert!(merge_ok(source, target, "alias:: Y, X\n- t\n- s\n"));
    // `y` is `Y`: listing it again is a different alias list.
    assert!(!merge_ok(source, target, "alias:: Y, X, y\n- t\n- s\n"));
}

#[test]
fn a_merge_joins_a_free_property_and_refuses_a_duplicated_entry() {
    let (source, target) = ("join:: v\n- s\n", "- t\n");
    assert!(merge_ok(source, target, "join:: v\n- t\n- s\n"));
    // Negative control: a duplicated header entry.
    assert!(!merge_ok(source, target, "join:: v\njoin:: v\n- t\n- s\n"));
}

#[test]
fn a_clash_goes_to_the_leftover_and_a_dropped_leftover_line_is_refused() {
    let (source, target) = ("k:: a\nm:: c\n- s\n", "k:: b\nm:: d\n- t\n");
    let after = "k:: b\nm:: d\n- t\n- \n  k:: a\n  m:: c\n- s\n";
    assert!(merge_ok(source, target, after));
    // Negative control: one leftover line dropped.
    assert!(!merge_ok(
        source,
        target,
        "k:: b\nm:: d\n- t\n- \n  k:: a\n- s\n"
    ));
}

#[test]
fn header_children_follow_the_leftover_and_a_reordered_block_is_refused() {
    let (source, target) = ("- k:: a\n  - kid\n- s\n", "k:: b\n- t\n");
    let after = "k:: b\n- t\n- \n  k:: a\n  - kid\n- s\n";
    assert!(merge_ok(source, target, after));
    // Negative control: the source's blocks before the target's.
    assert!(!merge_ok(
        source,
        target,
        "k:: b\n- \n  k:: a\n  - kid\n- s\n- t\n"
    ));
}

#[test]
fn a_survivor_header_may_be_permuted_and_nothing_else() {
    let (source, target) = ("j:: J\n- s\n", "x:: X\n- t\n");
    assert!(merge_ok(source, target, "j:: J\nx:: X\n- t\n- s\n"));
    let rename = Rename::new("A", "B", None, NAMES);
    let content = md("x:: X\ny:: Y\n- t\n");
    let want = expected(
        &rename,
        Format::Md,
        Expect::File {
            content: &content,
            skip: false,
        },
    );
    // A file that is no survivor keeps its header order.
    assert!(!matches(&want, &md("y:: Y\nx:: X\n- t\n"), false));
    assert!(matches(&want, &md("y:: Y\nx:: X\n- t\n"), true));
}

#[test]
fn org_links_values_and_titles_are_respelled() {
    let org = Format::Org;
    assert!(rename_ok(
        "Old",
        "New",
        "* see [[Old]]\n",
        "* see [[New]]\n",
        org
    ));
    assert!(!rename_ok(
        "Old",
        "New",
        "* see [[Old]]\n",
        "* see [[Old]]\n",
        org
    ));
    let before = "* h\n:PROPERTIES:\n:tags: Old\n:END:\n";
    let after = "* h\n:PROPERTIES:\n:tags: New\n:END:\n";
    assert!(rename_ok("Old", "New", before, after, org));
    let before = "#+title: Old/sub\n* x\n";
    assert!(rename_ok(
        "Old",
        "New",
        before,
        "#+title: New/sub\n* x\n",
        org
    ));
    assert!(!rename_ok(
        "Old",
        "New",
        before,
        "#+title: Old/sub\n* x\n",
        org
    ));
}

fn org_merge_ok(source: &str, target: &str, after: &str) -> bool {
    let org = Format::Org;
    matches(&merged(source, target, org), &observe(after, org), true)
}

#[test]
fn an_org_merge_joins_quotes_the_leftover_and_unites_aliases() {
    let (source, target) = ("#+key: a\n#+join: v\n* s\n", "#+key: b\n* t\n");
    let after = "#+key: b\n#+join: v\n* t\n* \n#+BEGIN_QUOTE\n#+key: a\n#+END_QUOTE\n* s\n";
    assert!(org_merge_ok(source, target, after));
    // Negative controls: the leftover line dropped; a block lost.
    let dropped = "#+key: b\n#+join: v\n* t\n* \n#+BEGIN_QUOTE\n#+END_QUOTE\n* s\n";
    assert!(!org_merge_ok(source, target, dropped));
    let lost = "#+key: b\n#+join: v\n* t\n* \n#+BEGIN_QUOTE\n#+key: a\n#+END_QUOTE\n";
    assert!(!org_merge_ok(source, target, lost));
    let (source, target) = ("#+alias: X\n* s\n", "#+alias: Y\n* t\n");
    assert!(org_merge_ok(source, target, "#+alias: Y, X\n* t\n* s\n"));
    assert!(!org_merge_ok(source, target, "#+alias: Y\n* t\n* s\n"));
}

#[test]
fn a_conflicted_file_that_stays_is_skipped() {
    let text = "<<<<<<< a\n=======\n>>>>>>> b\n- [[Old]]\n";
    assert!(skip_file(text, Format::Md, "pages/R.md", "pages/R.md"));
    assert!(!skip_file(text, Format::Md, "pages/R.md", "pages/S.md"));
    assert!(!skip_file(
        "- [[Old]]\n",
        Format::Md,
        "pages/R.md",
        "pages/R.md"
    ));
}

#[test]
fn expected_path_moves_only_a_renamed_page() {
    let rename = Rename::new("Old", "New/x", None, FileNameFormat::Legacy);
    assert_eq!(
        expected_path(&rename, "pages/Old.md", "Old", false),
        "pages/New%2Fx.md"
    );
    assert_eq!(
        expected_path(&rename, "pages/Old.md", "Old", true),
        "pages/Old.md"
    );
    assert_eq!(
        expected_path(&rename, "pages/R.org", "R", false),
        "pages/R.org"
    );
}

/// Q-c: the header key is the parser's key, also for an OG built-in key, which
/// has no key site. Two different built-in keys both join; the same key clashes.
#[test]
fn built_in_header_keys_join_by_the_parsers_key() {
    let (source, target) = ("icon:: 🌟\npublic:: true\n- s\n", "- t\n");
    assert!(merge_ok(
        source,
        target,
        "icon:: 🌟\npublic:: true\n- t\n- s\n"
    ));
    // Negative control: the second built-in entry sent to the leftover (the
    // r6 Lean's reading, where both keys are "").
    assert!(!merge_ok(
        source,
        target,
        "icon:: 🌟\n- t\n- \n  public:: true\n- s\n"
    ));
    let (source, target) = ("icon:: 🌟\n- s\n", "icon:: 🙂\n- t\n");
    assert!(merge_ok(
        source,
        target,
        "icon:: 🙂\n- t\n- \n  icon:: 🌟\n- s\n"
    ));
    assert!(!merge_ok(
        source,
        target,
        "icon:: 🙂\nicon:: 🌟\n- t\n- s\n"
    ));
}

/// Q-d: a property key is a typed site and follows the whole name map, its
/// namespace prefix included (model O1: `a/x::` names `A/x`, which `A` → `B`
/// moves to `B/x`).
#[test]
fn a_namespace_property_key_follows_the_rename() {
    let before = "- a/x:: v\n";
    assert!(rename_ok("A", "B", before, "- b/x:: v\n", Format::Md));
    // Negative control: the key matched only exactly, so left as it was.
    assert!(!rename_ok("A", "B", before, before, Format::Md));
    let before = "* h\n:PROPERTIES:\n:a/x: v\n:END:\n";
    assert!(rename_ok(
        "A",
        "B",
        before,
        "* h\n:PROPERTIES:\n:b/x: v\n:END:\n",
        Format::Org
    ));
    assert!(!rename_ok("A", "B", before, before, Format::Org));
}

/// A renamed key site renames the entry's key, so a merge compares the new
/// key: the source's `old::` becomes `new::` and clashes with the survivor's.
#[test]
fn a_renamed_key_clashes_under_its_new_key() {
    let rename = Rename::new("A", "B", None, NAMES);
    let want = expected(
        &rename,
        Format::Md,
        Expect::Survivor {
            source: &md("a:: 1\n- s\n"),
            target: &md("b:: 2\n- t\n"),
        },
    );
    assert!(matches(&want, &md("b:: 2\n- t\n- \n  b:: 1\n- s\n"), true));
    assert!(!matches(&want, &md("b:: 2\nb:: 1\n- t\n- s\n"), true));
}

/// Review round 2, finding 2: preamble prose reads with the parser's line
/// endings (LF), so a CRLF or lone-CR file reads as its LF twin, and a merge
/// or rename may change a file's line endings.
#[test]
fn preamble_prose_reads_with_normalized_line_endings() {
    // The prose reads as the block the outline makes of it, without the
    // newline that ends it (SPEC P4: line endings and blank lines are not
    // observed; GUARANTEES: no separate preamble observation), so the merged
    // block is `- Intro` and no blank continuation line follows it.
    let survivor = "- target\n- Intro\n- source\n";
    for source in [
        "Intro\n- source\n",
        "Intro\r\n- source\r\n",
        "Intro\r- source\r",
        "Intro\n\n- source\n",
    ] {
        assert!(merge_ok(source, "- target\n", survivor), "{source:?}");
        // Negative control: the preamble lost.
        assert!(
            !merge_ok(source, "- target\n", "- target\n- source\n"),
            "{source:?}"
        );
    }
    // Negative controls: a real line of the prose dropped, and the blank line
    // between two prose lines dropped.
    let (two, gap) = ("Intro\nmore\n- source\n", "Intro\n\nmore\n- source\n");
    assert!(merge_ok(
        two,
        "- target\n",
        "- target\n- Intro\n  more\n- source\n"
    ));
    assert!(!merge_ok(two, "- target\n", survivor));
    assert!(merge_ok(
        gap,
        "- target\n",
        "- target\n- Intro\n  \n  more\n- source\n"
    ));
    assert!(!merge_ok(
        gap,
        "- target\n",
        "- target\n- Intro\n  more\n- source\n"
    ));
    for before in [
        "Intro [[Old]]\r\n\r\n- x\r\n",
        "Intro [[Old]]\r\r- x\r",
        "Intro [[Old]]\n\n- x\n",
    ] {
        assert_eq!(
            md(before),
            md(&before.replace("\r\n", "\n").replace('\r', "\n"))
        );
        for after in [
            "Intro [[New]]\n\n- x\n",
            "Intro [[New]]\r\n\r\n- x\r\n",
            "Intro [[New]]\r\r- x\r",
        ] {
            assert!(
                rename_ok("Old", "New", before, after, Format::Md),
                "{before:?} → {after:?}"
            );
        }
        assert!(
            !rename_ok("Old", "New", before, before, Format::Md),
            "{before:?}"
        );
    }
}

/// Manager ruling Q1: the check reads text before the first block exactly as
/// the outline's block of it (`page_properties::preamble_block`, the text the
/// page shows and promotes), with no rule of its own: 0, 1 and 2 blank lines
/// before the first block, CRLF, and a file ending in the prose with no final
/// newline, Markdown and Org.
#[test]
fn preamble_prose_reads_as_the_outlines_block() {
    for (format, bullet) in [(Format::Md, "- x"), (Format::Org, "* x")] {
        let org = format == Format::Org;
        let texts = [
            format!("Intro\nmore\n{bullet}\n"),
            format!("Intro\nmore\n\n{bullet}\n"),
            format!("Intro\nmore\n\n\n{bullet}\n"),
            format!("Intro\r\nmore\r\n\r\n{bullet}\r\n"),
            "Intro\nmore".to_owned(),
        ];
        for text in &texts {
            let doc = match org {
                true => crate::org::parse_org(text),
                false => crate::doc::parse(text),
            };
            let pre = doc.pre_block.as_deref().expect("prose is a pre-block");
            let outline = crate::page_properties::preamble_block(pre, org).expect("prose");
            assert_eq!(outline.raw(), "Intro\nmore", "{text:?}");
            let reading = observe(text, format);
            assert_eq!(
                reading.blocks[0].body,
                vec![Atom::Text(outline.raw().to_owned())],
                "{format:?} {text:?}"
            );
        }
    }
}

/// Steps (sites visited, keys, identities and header entries compared) for
/// `f`, from a reset counter.
fn steps(f: impl FnOnce()) -> usize {
    STEPS.with(|n| n.set(0));
    f();
    STEPS.with(|n| n.get())
}

/// Review round 2, finding 4: a merge whose source has 10,000 header
/// properties costs steps linear in them, for the survivor in order and
/// reversed (a permuted header is valid).
#[test]
fn many_header_properties_cost_linear_steps() {
    let n = 10_000;
    let props: Vec<String> = (0..n).map(|i| format!("p{i:05}:: v\n")).collect();
    let source = format!("{}- source\n", props.concat());
    let in_order = format!("{}- target\n- source\n", props.concat());
    let reversed: String = props.iter().rev().map(String::as_str).collect();
    let reversed = format!("{reversed}- target\n- source\n");
    for after in [in_order, reversed] {
        let mut ok = false;
        let taken = steps(|| ok = merge_ok(&source, "- target\n", &after));
        assert!(ok);
        assert!(taken <= 20 * n, "{taken} steps for {n} properties");
    }
}

/// A merge uniting 10,000 distinct aliases costs steps linear in them.
#[test]
fn many_aliases_cost_linear_steps() {
    let n = 10_000;
    let names: Vec<String> = (0..n).map(|i| format!("a{i:04}")).collect();
    let source = format!("alias:: {}\n- source\n", names.join(", "));
    let after = format!("alias:: {}\n- target\n- source\n", names.join(", "));
    let mut ok = false;
    let taken = steps(|| ok = merge_ok(&source, "- target\n", &after));
    assert!(ok);
    assert!(taken <= 20 * n, "{taken} steps for {n} aliases");
}

/// One query macro with 1,000 page-name arguments: its observation holds the
/// macro once (all its sites share it), so the observation's size is linear
/// in the text; the writer's spelling of every argument passes.
#[test]
fn many_query_arguments_share_one_macro() {
    let n = 1_000;
    let name = crate::query::ir::QUERY_MACRO_NAMES[0];
    let before = format!("- {{{{{name} (or {})}}}}\n", "[[A]] ".repeat(n));
    let after = before.replace("[[A]]", "[[B]]");
    let c = md(&before);
    let mut seen = std::collections::HashSet::new();
    let mut held = 0;
    for s in c.blocks.iter().flat_map(|b| text_sites(&b.body)) {
        if let Some((q, _)) = &s.within {
            if seen.insert(Arc::as_ptr(q)) {
                held += q.argument.len();
            }
        }
    }
    assert_eq!(
        c.blocks[0]
            .body
            .iter()
            .filter(|a| matches!(a, Atom::Site(_)))
            .count(),
        n
    );
    assert!(
        held <= 2 * before.len(),
        "{held} bytes of macro for a {}-byte text",
        before.len()
    );
    assert!(rename_ok("A", "B", &before, &after, Format::Md));
    assert!(!rename_ok("A", "B", &before, &before, Format::Md));
}

/// `observe` parses the document once, whatever the text holds.
#[test]
fn observe_parses_the_document_once() {
    for (text, format) in [
        (
            "title:: A\nalias:: B\nprose\n\n- [[x]]\n  - y\n",
            Format::Md,
        ),
        ("- k:: v\n  - kid\n- z\n", Format::Md),
        ("#+title: A\n* h :t:\n** k\n", Format::Org),
    ] {
        observe::DOCUMENT_PARSES.with(|n| n.set(0));
        observe(text, format);
        assert_eq!(observe::DOCUMENT_PARSES.with(|n| n.get()), 1, "{text:?}");
    }
}

/// O6: "any other header text are the leftover lines". Org prose before a
/// directive is header text: it was dropped from the reading, so a writer
/// losing it passed and the writer keeping it was refused.
#[test]
fn org_prose_before_a_directive_is_other_header_text() {
    let source = "free text\n\n#+k: s\n* source\n";
    let want = merged(source, "#+k: t\n* target\n", Format::Org);
    let read = |text: &str| matches(&want, &observe(text, Format::Org), true);
    assert!(read(
        "#+k: t\n* target\n*\n#+BEGIN_QUOTE\nfree text\n#+k: s\n#+END_QUOTE\n* source\n"
    ));
    // Negative control: the prose lost.
    assert!(!read(
        "#+k: t\n* target\n*\n#+BEGIN_QUOTE\n#+k: s\n#+END_QUOTE\n* source\n"
    ));
}

/// An Org head drawer's alias declares its names, as it does when another
/// drawer follows: the header is read through the drawer's `:END:` (found by
/// the transaction check, where a lone drawer's `:ALIAS:` read as other
/// header text and every merge keeping it was refused).
#[test]
fn a_lone_org_drawer_alias_declares_its_names() {
    let names = |text: &str| super::semantic(&observe(text, Format::Org)).alias_names;
    assert_eq!(
        names(":PROPERTIES:\n:ALIAS: a, b\n:END:\n* t\n"),
        ["a", "b"]
    );
    assert_eq!(
        names(":PROPERTIES:\n:ALIAS: a\n:END:\n:PROPERTIES:\n:ALIAS: b\n:END:\n* t\n"),
        ["a", "b"]
    );
    let source = ":PROPERTIES:\n:ALIAS: x\n:END:\n* source\n";
    let want = merged(source, "* target\n", Format::Org);
    let read = |text: &str| matches(&want, &observe(text, Format::Org), true);
    assert!(read(":PROPERTIES:\n:ALIAS: x\n:END:\n* target\n* source\n"));
    // Negative controls: the alias lost, or written as another name.
    assert!(!read("* target\n* source\n"));
    assert!(!read(
        ":PROPERTIES:\n:ALIAS: c\n:END:\n* target\n* source\n"
    ));
}

/// O6: "Text before the first block of a file is read as that file's first
/// block": a query in preamble prose is a query site, as in a block.
#[test]
fn a_query_in_preamble_prose_reads_as_in_a_block() {
    let within = |c: &Content| {
        text_sites(&c.blocks[0].body)
            .map(|s| (s.kind, s.within.as_ref().map(|w| w.0.name.clone())))
            .collect::<Vec<_>>()
    };
    let block = md("- {{query [[A]]}}\n- x\n");
    assert_eq!(within(&md("{{query [[A]]}}\n- x\n")), within(&block));
    assert_eq!(
        within(&md("title:: t\n{{query [[A]]}}\n- x\n")),
        within(&block)
    );
}
