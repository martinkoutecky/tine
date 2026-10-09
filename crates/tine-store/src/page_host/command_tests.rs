//! STEP3 §3: typed outcomes and the save's twin checks (Q9).
use super::io::Phase;
use super::model_fs::Fault;
use super::tests::{drain, edit, host, open, send, text};
use super::*;

fn refusals(h: &Host<model_fs::ModelFs>) -> Vec<(PageKey, Refusal)> {
    h.events
        .iter()
        .filter_map(|event| match event {
            Event::Refused { page, reason, .. } => Some((page.clone(), *reason)),
            _ => None,
        })
        .collect()
}

/// §3.3: every answer that did not take its request names why, so the UI
/// can say "discard failed" or "moved text was not kept".
#[test]
fn an_answer_that_did_not_take_its_request_names_why() {
    // A discard whose read failed.
    let mut h = host();
    open(&mut h, "a.md");
    edit(&mut h, "a.md", "mine");
    let version = h.pages["a.md"].version;
    h.fs.inject(Phase::Read, [Fault::Before]);
    send(&mut h, "a.md", RequestKind::Discard { version });
    assert!(!h.outbox["a.md"].answer.as_ref().unwrap().took);
    assert_eq!(refusals(&h), vec![("a.md".into(), Refusal::ReadFailed)]);
    assert_eq!(h.pages["a.md"].buf, text("mine"));

    // A submit or discard for a page the host does not hold.
    let mut h = host();
    let submit = RequestKind::Submit {
        bytes: text("x"),
        version: 0,
        resolve: None,
    };
    assert_eq!(send(&mut h, "a.md", submit), Disposition::Refused);
    send(&mut h, "b.md", RequestKind::Discard { version: 0 });
    assert_eq!(
        refusals(&h),
        vec![
            ("a.md".into(), Refusal::NotHeld),
            ("b.md".into(), Refusal::NotHeld)
        ]
    );

    // A move whose versions are no longer current.
    let mut h = host();
    open(&mut h, "a.md");
    open(&mut h, "b.md");
    let a = h.pages["a.md"].version;
    let b = h.pages["b.md"].version;
    edit(&mut h, "b.md", "newer");
    let stale = RequestKind::Move {
        receiver: "b.md".into(),
        source_text: text("rest"),
        receiver_text: text("moved"),
        source_version: a,
        receiver_version: b,
    };
    assert_eq!(send(&mut h, "a.md", stale), Disposition::Refused);
    assert_eq!(
        refusals(&h),
        vec![
            ("b.md".into(), Refusal::Stale),
            ("a.md".into(), Refusal::Stale)
        ]
    );

    // A move whose receiver draft could not be written.
    let mut h = host();
    open(&mut h, "a.md");
    open(&mut h, "b.md");
    let a = h.pages["a.md"].version;
    let b = h.pages["b.md"].version;
    h.fs.inject(Phase::DraftTemp, [Fault::Before; 3]);
    h.fs.inject(Phase::DraftSync, [Fault::Before; 3]);
    let moving = RequestKind::Move {
        receiver: "b.md".into(),
        source_text: text("rest"),
        receiver_text: text("moved"),
        source_version: a,
        receiver_version: b,
    };
    assert_eq!(send(&mut h, "a.md", moving), Disposition::Pending);
    drain(&mut h);
    assert!(!h.outbox["a.md"].answer.as_ref().unwrap().took);
    assert_eq!(
        refusals(&h),
        vec![
            ("b.md".into(), Refusal::DraftFailed),
            ("a.md".into(), Refusal::DraftFailed)
        ]
    );
}

fn twins(h: &Host<model_fs::ModelFs>) -> Vec<String> {
    h.events
        .iter()
        .filter_map(|event| match event {
            Event::Twin { existing, .. } => Some(existing.clone()),
            _ => None,
        })
        .collect()
}

fn outcome(h: &Host<model_fs::ModelFs>) -> Option<Outcome> {
    h.events.iter().rev().find_map(|event| match event {
        Event::SaveOutcome { outcome, .. } => Some(*outcome),
        _ => None,
    })
}

/// §3.2: a twin present at the Check phase fails a creating save before the
/// rename, as the model's ordinary failed save: nothing is written, and the
/// page stays at risk.
#[test]
fn a_twin_before_the_rename_fails_the_creating_save() {
    let mut h = host();
    open(&mut h, "c.md");
    edit(&mut h, "c.md", "created");
    h.fs.twins.insert("c.md".into(), "c.org".into());
    assert_eq!(h.start_save("c.md"), Disposition::Pending);
    while h.job.is_some() {
        h.advance_save(0);
    }
    assert_eq!(outcome(&h), Some(Outcome::Failed));
    assert_eq!(twins(&h), vec!["c.org".to_string()]);
    assert!(!h.fs.files.contains_key("graph/c.md"));
    assert!(h.pages["c.md"].risk);
}

/// Q9: a twin delivered after the Check phase is found right after the
/// rename. It is a notice beside the save's own Published outcome; the
/// host undoes nothing, so both files stay for the user to settle.
#[test]
fn a_twin_after_the_rename_is_a_notice_and_the_save_still_publishes() {
    let mut h = host();
    open(&mut h, "c.md");
    edit(&mut h, "c.md", "created");
    assert_eq!(h.start_save("c.md"), Disposition::Pending);
    while h.job.as_ref().unwrap().phase != SavePhase::Rename {
        h.advance_save(0);
    }
    h.fs.twins.insert("c.md".into(), "c.org".into());
    while h.job.is_some() {
        h.advance_save(0);
    }
    assert_eq!(outcome(&h), Some(Outcome::Published));
    assert_eq!(twins(&h), vec!["c.org".to_string()]);
    assert_eq!(h.fs.files["graph/c.md"].as_ref(), b"created");
    assert!(h.pages["c.md"].clean());
}

/// A save that replaces an existing file is no creation: no twin probe.
#[test]
fn a_replacing_save_does_not_probe_for_a_twin() {
    let mut h = host();
    open(&mut h, "a.md");
    edit(&mut h, "a.md", "replaced");
    h.fs.twins.insert("a.md".into(), "a.org".into());
    assert_eq!(h.start_save("a.md"), Disposition::Pending);
    while h.job.is_some() {
        h.advance_save(0);
    }
    assert_eq!(outcome(&h), Some(Outcome::Published));
    assert!(twins(&h).is_empty());
}

/// STEP3 §1: a command thread runs delete and rename as plan, lock,
/// revalidate. With no path lock held they name the locks they need and
/// return before any side effect; holding them, they proceed.
#[test]
fn delete_and_rename_plan_their_locks_before_any_side_effect() {
    let mut h = host();
    open(&mut h, "a.md");
    h.held = Some(BTreeSet::new());
    let before = (h.events.len(), h.fs.files.clone());
    assert_eq!(h.delete("a.md"), Disposition::Waiting);
    assert_eq!(
        h.lock_request.take(),
        Some(BTreeSet::from(["a.md".to_string()]))
    );
    let rename = |h: &mut Host<model_fs::ModelFs>| {
        h.rename(
            "a.md",
            "c.md",
            &BTreeSet::new(),
            "A",
            "C",
            tine_core::config::FileNameFormat::TripleLowbar,
        )
    };
    assert_eq!(rename(&mut h), Disposition::Waiting);
    let wanted = h.lock_request.take().unwrap();
    assert!(wanted.contains("a.md") && wanted.contains("c.md"));
    assert_eq!((h.events.len(), h.fs.files.clone()), before);
    assert!(h.pages.contains_key("a.md") && !h.pages.contains_key("c.md"));

    h.held = Some(wanted);
    assert_eq!(rename(&mut h), Disposition::Pending);
    assert!(h.lock_request.is_none());
}
