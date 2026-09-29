//! T1 (og-t): the page title is read from the preamble the page model reads
//! (`doc::parse` / `org::parse_org` `pre_block`), and a line-streaming reader
//! that stops where `preamble_read` says `Settled` gets the same answer as a
//! reader of the whole file.
use tine_core::model::{page_title_from_preamble, preamble_read, Format, PreambleRead};

const SHAPES: &[(&str, Format, Option<&str>)] = &[
    ("title:: Plain\n\n- body\n", Format::Md, Some("Plain")),
    (
        "# Heading\ntitle:: Heading Prop\n\n- body\n",
        Format::Md,
        None,
    ),
    (
        "```\n- not a block\n```\ntitle:: Fenced\n- body\n",
        Format::Md,
        Some("Fenced"),
    ),
    ("```\n- x\n```\n", Format::Md, None),
    (
        "#+BEGIN_SRC\n- x\n#+END_SRC\ntitle:: Src\n- b\n",
        Format::Md,
        Some("Src"),
    ),
    (
        "alias:: a\n#tag line\ntitle:: After Tag\n- b\n",
        Format::Md,
        Some("After Tag"),
    ),
    ("title:: Crlf\r\n\r\n- body\r\n", Format::Md, Some("Crlf")),
    (
        "alias:: a\rtitle:: Lone Cr\r- body\r",
        Format::Md,
        Some("Lone Cr"),
    ),
    ("- first\ntitle:: Too Late\n", Format::Md, None),
    ("title:: No Blocks\n", Format::Md, Some("No Blocks")),
    ("#+title: Org Title\n* h\n", Format::Org, Some("Org Title")),
    (
        "#+BEGIN_SRC\n* x\n#+END_SRC\n#+title: Org Src\n* r\n",
        Format::Org,
        Some("Org Src"),
    ),
    (
        ":PROPERTIES:\n:title: Drawer\n:END:\n* h\n",
        Format::Org,
        Some("Drawer"),
    ),
    ("* h\n#+title: Late\n", Format::Org, None),
];

fn streamed(text: &str, format: Format) -> String {
    let mut prefix = String::new();
    for line in text.split_inclusive(['\n']) {
        prefix.push_str(line);
        match preamble_read(&prefix, format) {
            PreambleRead::Settled => return prefix,
            PreambleRead::More => {}
            PreambleRead::Whole => return text.to_owned(),
        }
    }
    prefix
}

#[test]
fn title_follows_the_model_preamble_and_a_streamed_prefix_agrees() {
    for (text, format, expected) in SHAPES {
        let whole = page_title_from_preamble(text, *format);
        assert_eq!(whole.as_deref(), *expected, "whole file: {text:?}");
        let prefix = streamed(text, *format);
        assert_eq!(
            page_title_from_preamble(&prefix, *format),
            whole,
            "streamed prefix {prefix:?} of {text:?}"
        );
        let model_pre = match format {
            Format::Md => tine_core::doc::parse(text).pre_block,
            Format::Org => tine_core::org::parse_org(text).pre_block,
        };
        if expected.is_some() {
            assert!(
                model_pre.is_some(),
                "a title lives in the model preamble: {text:?}"
            );
        }
    }
}

#[test]
fn a_property_only_preamble_settles_at_the_first_bullet() {
    let text = "title:: Early\nalias:: e\n\n- first\n- second\n";
    let prefix = streamed(text, Format::Md);
    assert_eq!(prefix, "title:: Early\nalias:: e\n\n- first\n");
}
