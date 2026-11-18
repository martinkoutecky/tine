//! [`unchanged_save`] is what the real save writes for a page loaded and saved
//! with no edit, and its work per call is counted.

use super::*;
use crate::test_host_writer::HostWriter;
use crate::{EditKind, PageId, SaveBase, Store};

/// Inputs of the existing save tests (`tests/save_layout_retention.rs`, the
/// A5, GH #163, lone-CR and Org round-trip tests in `model.rs`), plus the
/// unstable first-bullet referrer the model names.
const MARKDOWN: &[&str] = &[
    "- a\n- \n",
    "title:: Fix\ntags:: a\n\n\t- one\n\t  cont  \n\t  \n\t\t\t- deep\n\t- two\n\t\t- child\n\t- three",
    "title:: Fix\r\ntags:: a\r\n\r\n\t- one\r\n\t  cont  \r\n\t  \r\n\t\t\t- deep\r\n\t- two\r\n\t\t- child\r\n\t- three\r\n",
    "- a\n- b\n\n\n",
    "- a\n-\n\n",
    "x:: y\n\n- a\n\n\n\n",
    "- open\n  ```js\n  code\n",
    "- a\r\n  \r\n  b\r\n   \r\n  c\r\n- d\r\n",
    "- a\r- b\r",
    "title:: solo\n",
    "title:: solo",
    "- editable root\n## first section\n### second section\n- trailing root",
    "# Project\n\t- child one\n\t- child two\n- sibling",
    "-\tx\n- \n-\n-  y\n- z",
    "A:: XX\nB:: XX\nC:: XX\n",
    "- tags:: [[Old]]\n",
];

const ORG: &[&str] = &[
    "* TODO Buy milk\nSCHEDULED: <2026-06-25 Thu>\n* second block\n",
    "#+title: A\n* source\n** child\n",
    "* a\r\n* b\r\n",
    "* a\n*** c\n",
];

fn pure(rel: &str, text: &str) -> io::Result<String> {
    let path = Path::new(rel);
    unchanged_save(path, Some(rel), text, parse_doc_with_opts(path, text))
}

/// What a real load and save with no edit leaves on disk, `None` if refused.
fn real(rel: &str, text: &str) -> Option<String> {
    let dir = crate::test_support::fresh_dir("tine-unchanged-save-");
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::write(dir.join(rel), text).unwrap();
    let store = Arc::new(Store::open(&dir, Default::default()).unwrap().0);
    let id = PageId::from(rel);
    // A page the store does not load is not saved either.
    let out = store.page(&id).ok().and_then(|read| {
        HostWriter::start(&store)
            .save(
                EditKind::ReplacePage,
                &id,
                SaveBase::Existing(read.rev),
                &read.doc,
            )
            .ok()
            .map(|_| fs::read_to_string(dir.join(rel)).unwrap())
    });
    store.close();
    let _ = fs::remove_dir_all(&dir);
    out
}

#[test]
fn the_pure_function_writes_what_the_real_save_writes() {
    let cases = MARKDOWN
        .iter()
        .map(|t| ("pages/A.md", *t))
        .chain(ORG.iter().map(|t| ("pages/A.org", *t)));
    let mut refused = Vec::new();
    for (rel, text) in cases {
        let written = real(rel, text);
        assert_eq!(pure(rel, text).ok(), written, "{text:?}");
        if written.is_none() {
            refused.push(text);
        }
    }
    // Only the Org file that does not round-trip is refused, by both.
    assert_eq!(refused, ["* a\n*** c\n"]);
}

#[test]
fn a_childless_properties_only_first_bullet_does_not_survive() {
    let text = "- tags:: [[Old]]\n";
    assert_eq!(pure("pages/R.md", text).unwrap(), "tags:: [[Old]]\n");
    // Its stable spelling, and an ordinary first bullet, come back unchanged.
    for stable in [
        "tags:: [[Old]]\n",
        "- [[Old]]\n",
        "- tags:: [[Old]]\n  - kid\n",
    ] {
        assert_eq!(pure("pages/R.md", stable).unwrap(), stable);
    }
}

#[test]
fn an_org_file_that_does_not_round_trip_is_refused() {
    let refused = pure("pages/A.org", "* a\n*** c\n").unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied);
}

/// How a save refuses, as the real save's `Why` and the pure function's
/// `io::Error` both say it: the family of refusal and its message.
#[derive(Debug, PartialEq)]
enum Refused {
    /// Read-only: VCS conflict markers, an Org file that does not round-trip,
    /// a parser panic.
    ReadOnly(String),
    /// The content cannot be parsed safely: size, nesting.
    Unparseable(String),
    /// The bytes are not UTF-8.
    Undecodable,
}

fn pure_refusal(rel: &str, text: &str) -> Option<Refused> {
    let error = pure(rel, text).err()?;
    let message = error.to_string();
    let not_utf8 = error.get_ref().is_some_and(|e| e.is::<NotUtf8>());
    let panicked = error.get_ref().is_some_and(|e| e.is::<ParserPanicked>());
    Some(match error.kind() {
        _ if not_utf8 => Refused::Undecodable,
        io::ErrorKind::PermissionDenied => Refused::ReadOnly(message),
        _ if panicked => Refused::ReadOnly(message),
        io::ErrorKind::InvalidData => Refused::Unparseable(message),
        kind => panic!("unexpected refusal kind {kind:?}: {message}"),
    })
}

/// What the real unchanged save says of page bytes `old` at `rel`: the
/// transaction's own serializer (`serialize_host_input`, the one place that
/// serializes page bytes for a save) over `old`, given the DTO of a loadable
/// page of the same format (every refusal below is a verdict on `old`).
fn real_refusal(rel: &str, old: &[u8]) -> Option<Refused> {
    use crate::transaction::{Refusal, Why};
    let dir = crate::test_support::fresh_dir("tine-unchanged-save-");
    fs::create_dir_all(dir.join("pages")).unwrap();
    let ext = Path::new(rel).extension().unwrap().to_str().unwrap();
    let good = format!("pages/Good.{ext}");
    let content = if ext == "org" {
        "* editable\n"
    } else {
        "- editable\n"
    };
    fs::write(dir.join(&good), content).unwrap();
    let store = Arc::new(Store::open(&dir, Default::default()).unwrap().0);
    let id = PageId::from(good.as_str());
    let doc = store.page(&id).unwrap().doc;
    let out = store
        .page_plan(None)
        .serialize_host_input(&id.file(), &doc, Some(old));
    store.close();
    let _ = fs::remove_dir_all(&dir);
    match out {
        Ok(_) => None,
        Err(Why::Refused(Refusal::ReadOnly(message))) => Some(Refused::ReadOnly(message)),
        Err(Why::Refused(Refusal::InvalidTarget(message))) => Some(Refused::Unparseable(
            message
                .strip_prefix("page content cannot be parsed safely: ")
                .expect("the content refusal's wording")
                .to_owned(),
        )),
        Err(Why::Refused(Refusal::Undecodable)) => Some(Refused::Undecodable),
        Err(other) => panic!("not a refusal: {other:?}"),
    }
}

/// Review round 2, finding 3: the pure function refuses what the real
/// unchanged save refuses, family by family, with the same refusal and the
/// same message (both run `guarded_save`'s firewalls and `parser_unwind`):
/// unresolved VCS conflict markers (Markdown and Org), the parse size limit,
/// an outline nested past the parse limit, an Org headline past it, a parser
/// panic, and an Org file that does not round-trip. Text that is not UTF-8
/// reaches the real save only (the pure function takes `&str`, and the check
/// refuses such bytes before it, `model_check::check`).
#[test]
fn every_refusal_family_of_the_real_save_is_the_pure_functions() {
    let deep: String = (0..130)
        .map(|d| format!("{}- d{d}\n", "  ".repeat(d)))
        .collect();
    let deep_org: String = format!("{} deep\n", "*".repeat(130));
    let huge = format!("- {}", "a".repeat(PARSE_INPUT_MAX_BYTES as usize));
    let panics = format!("- {}\n", tine_core::doc::TEST_PROJECTION_PANIC_SENTINEL);
    let cases: Vec<(&str, &str)> = vec![
        (
            "pages/R.md",
            "<<<<<<< ours\nx\n=======\ny\n>>>>>>> theirs\n- kept\n",
        ),
        (
            "pages/R.md",
            "- a\n<<<<<<< ours\n- x\n=======\n- y\n>>>>>>> theirs\n",
        ),
        (
            "pages/R.org",
            "* a\n<<<<<<< ours\n* x\n=======\n* y\n>>>>>>> theirs\n",
        ),
        ("pages/R.md", &deep),
        ("pages/R.org", &deep_org),
        ("pages/R.md", &huge),
        ("pages/R.md", &panics),
        ("pages/R.org", "* a\n*** c\n"),
    ];
    for (rel, text) in cases {
        let shown: String = text.chars().take(60).collect();
        let real = real_refusal(rel, text.as_bytes());
        assert!(real.is_some(), "the real save takes {shown:?}");
        assert_eq!(pure_refusal(rel, text), real, "{shown:?}");
    }
    // Not UTF-8: the real save's answer, which no `&str` can ask the pure one.
    assert_eq!(
        real_refusal("pages/R.md", b"- a\xff\n"),
        Some(Refused::Undecodable)
    );
}

/// Per call, on top of the caller's one parse of the candidate: Markdown
/// renders once and parses the render once (layout retention's own check);
/// Org serializes once (the round-trip firewall's bytes are the save's) and
/// parses once (the save's re-read of what it wrote).
#[test]
fn the_pure_function_parses_and_serializes_once() {
    for (rel, text) in [
        (
            "pages/A.md",
            "title:: A\nalias:: B\n\n- a [[x]]\n  - b\n- c\n",
        ),
        ("pages/A.md", "- a\r\n- b\r\n"),
        ("pages/A.org", "#+title: A\n* a\n** b\n* c\n"),
    ] {
        let path = Path::new(rel);
        let format = Format::from_path(path);
        SAVE_WORK.with(|w| w.set((0, 0)));
        // The one parse of the candidate, shared by the P2 reading and P3.
        let parsed = parse_doc_with_opts(path, text);
        let names = tine_core::config::FileNameFormat::TripleLowbar;
        let observed = tine_core::cm_check::observe_parsed(text, &parsed.0, format, names);
        assert_eq!(SAVE_WORK.with(|w| w.get()), (1, 0), "{text:?}");
        assert_eq!(observed, tine_core::cm_check::observe(text, format, names));
        SAVE_WORK.with(|w| w.set((0, 0)));
        assert_eq!(unchanged_save(path, Some(rel), text, parsed).unwrap(), text);
        assert_eq!(SAVE_WORK.with(|w| w.get()), (1, 1), "{text:?}");
    }
}
