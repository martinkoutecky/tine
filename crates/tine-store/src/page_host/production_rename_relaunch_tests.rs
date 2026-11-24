//! Run 2 (rename end states): a rename A to C over the referrer B is
//! admitted and its operation draft is durable and applied; the process dies
//! before C (the destination) publishes; a cold relaunch restores each page
//! from its draft with no order (SPEC-s3 "completion at relaunch only"); then
//! C's pending change is discarded and the saves run. The production host is
//! synchronous: dropping it without a stop is a process death (it writes
//! nothing it is not stepped through).
use super::*;

const OLD: &str = "- old\n";
const REF: &str = "- see [[A]]\n";
const REWRITTEN: &str = "- see [[C]]\n";

/// The admitted rename, durable and applied in memory, then a process death
/// before any page saved, then a cold relaunch.
fn relaunched_after_a_crash_before_the_destination_published() -> Fixture {
    let mut f = Fixture::new();
    fs::write(f.graph.join("a.md"), OLD).unwrap();
    fs::write(f.graph.join("b.md"), REF).unwrap();
    assert_eq!(
        f.host.rename(
            "a.md",
            "c.md",
            &BTreeSet::from(["b.md".into()]),
            "A",
            "C",
            tine_core::config::FileNameFormat::TripleLowbar
        ),
        Disposition::Pending
    );
    f.drain();
    assert_eq!(f.host.pages["c.md"].buf.as_deref(), Some(OLD.as_bytes()));
    assert!(!f.graph.join("c.md").exists(), "C is not published");
    // Crash: no stop, no save. Relaunch on the same app data.
    let f = f.cold_reopen();
    assert_eq!(f.host.pages["a.md"].buf, None, "A's deletion is recovered");
    assert_eq!(f.host.pages["c.md"].buf.as_deref(), Some(OLD.as_bytes()));
    assert_eq!(
        f.host.pages["b.md"].buf.as_deref(),
        Some(REWRITTEN.as_bytes())
    );
    assert!(
        !f.host.pages["c.md"].conflict,
        "a recovered C is not conflicted"
    );
    assert_eq!(f.host.fs.draft_files(true).len(), 3, "one draft per page");
    f
}

fn on_disk(f: &Fixture, key: &str) -> Option<String> {
    fs::read_to_string(f.graph.join(key)).ok()
}

fn trashed(f: &Fixture) -> Vec<String> {
    let mut texts: Vec<String> = fs::read_dir(&f.trash)
        .unwrap()
        .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
        .collect();
    texts.sort();
    texts
}

/// The draft steps the progress loop runs for settled pages: each page
/// whose draft no longer matches its buffer gets its removal (or rewrite).
fn retire_drafts(f: &mut Fixture) {
    for key in ["a.md", "b.md", "c.md"] {
        if f.host.begin_draft_retiring(key) == Disposition::Pending {
            f.drain();
        }
    }
}

fn pending_drafts(f: &Fixture) -> usize {
    f.host.fs.draft_files(true).len()
}

/// Run 2 through the host request (the `page_discard` command): a Discard
/// of the recovered, non-conflicted C. With no order after the relaunch the
/// Discard finds no partner, so A's deletion is not reverted: C takes its
/// disk state (no file) and its draft is dropped; A's recovered deletion
/// then saves and moves A to the Tine trash; B publishes its rewrite to C.
/// End: neither A nor C exists; A's text is only in
/// `logseq/.tine-trash/pages`; B links a page that does not exist.
// CHARACTERIZATION: current behaviour, not a ruling
// HARM: the page is left only in trash: A and C are both gone from the
// graph and the text survives only in logseq/.tine-trash/pages.
#[test]
fn a_discard_of_a_recovered_destination_leaves_the_source_only_in_trash() {
    let mut f = relaunched_after_a_crash_before_the_destination_published();
    f.send("c.md", RequestKind::Discard);
    assert!(f.host.pages["c.md"].clean(), "C took its disk state");
    assert_eq!(f.host.pages["c.md"].buf, None, "C: no file");
    assert_eq!(
        f.host.pages["a.md"].buf, None,
        "A's deletion is not reverted"
    );
    assert_eq!(f.host.start_save("c.md"), Disposition::Disabled);
    assert_eq!(f.save("a.md"), Outcome::Published);
    assert_eq!(f.save("b.md"), Outcome::Published);
    retire_drafts(&mut f);
    assert_eq!(on_disk(&f, "a.md"), None, "A is gone");
    assert_eq!(on_disk(&f, "c.md"), None, "C was never written");
    assert_eq!(on_disk(&f, "b.md").as_deref(), Some(REWRITTEN));
    assert_eq!(
        trashed(&f),
        vec![OLD.to_string()],
        "A's text: only in trash"
    );
    assert_eq!(pending_drafts(&f), 0, "no draft holds A's text");
}

/// Run 2's UI-reachable variant: the window offers a Discard (Use disk)
/// only on a conflicted page (`wiring.ts` `resolveConflict`). A recovered C
/// conflicts when a file appears at its path before its save (another
/// device's sync delivers `c.md`). A's recovered deletion is ungated after
/// the relaunch, so it publishes while C is conflicted; the user then takes
/// disk on C. End: A is gone, C holds the delivered text, A's text is only
/// in `logseq/.tine-trash/pages`, and no draft holds it.
// CHARACTERIZATION: current behaviour, not a ruling
// HARM: the page is left only in trash: A's text is in no graph file and
// no draft, only in logseq/.tine-trash/pages.
#[test]
fn a_take_disk_on_a_recovered_destination_conflicted_by_sync_leaves_the_source_only_in_trash() {
    let mut f = relaunched_after_a_crash_before_the_destination_published();
    fs::write(f.graph.join("c.md"), "- synced\n").unwrap();
    assert_eq!(f.host.observe("c.md"), Disposition::Applied);
    assert!(
        f.host.pages["c.md"].conflict,
        "C conflicts with the delivery"
    );
    assert_eq!(
        f.host.start_save("c.md"),
        Disposition::Disabled,
        "a conflicted page never saves"
    );
    assert_eq!(
        f.save("a.md"),
        Outcome::Published,
        "A's deletion is ungated"
    );
    assert_eq!(on_disk(&f, "a.md"), None);
    assert_eq!(trashed(&f), vec![OLD.to_string()]);
    // The user takes disk on C's conflict.
    f.send("c.md", RequestKind::Discard);
    assert_eq!(f.save("b.md"), Outcome::Published);
    retire_drafts(&mut f);
    assert_eq!(on_disk(&f, "a.md"), None, "A is gone");
    assert_eq!(on_disk(&f, "c.md").as_deref(), Some("- synced\n"));
    assert_eq!(on_disk(&f, "b.md").as_deref(), Some(REWRITTEN));
    assert_eq!(
        trashed(&f),
        vec![OLD.to_string()],
        "A's text: only in trash"
    );
    assert_eq!(pending_drafts(&f), 0, "no draft holds A's text");
}
