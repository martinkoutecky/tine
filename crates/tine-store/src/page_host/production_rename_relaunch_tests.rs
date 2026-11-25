//! A rename's pairing survives relaunch (regression `rename-cancel-relaunch`):
//! a rename A to C over the referrer B is admitted and its paired operation
//! draft is durable; the process dies before C (the receiver) publishes; a
//! cold relaunch reinstalls the pairing from the draft store before any save.
//! Use Disk on C then cancels the pair durably: A stays on disk, and no
//! later launch replays A's removal. The production host is synchronous:
//! dropping it without a stop is a process death (it writes nothing it is
//! not stepped through), so every cut below is "stop stepping here".
use super::*;

const OLD: &str = "- old\n";
const REF: &str = "- see [[A]]\n";
const REWRITTEN: &str = "- see [[C]]\n";
const KEYS: [&str; 5] = ["a.md", "b.md", "c.md", "d.md", "sub/a.md"];

fn bytes(text: &str) -> Text {
    Some(Arc::from(text.as_bytes()))
}

/// A cold relaunch on the same app data, with every key this file uses.
fn reopen(f: Fixture) -> Fixture {
    reopen_with(f, &[])
}

/// A cold relaunch whose launch meets `faults`.
fn reopen_with(f: Fixture, faults: &[Phase]) -> Fixture {
    reopen_syncing(f, faults, &[])
}

/// A cold relaunch whose launch meets `faults`, and whose syncs of the
/// directories in `syncs` keep failing with their error.
fn reopen_syncing(f: Fixture, faults: &[Phase], syncs: &[(PathBuf, io::ErrorKind)]) -> Fixture {
    let Fixture {
        graph,
        app,
        trash,
        host,
        root,
    } = f;
    drop(host);
    let mut io = ProductionIo::attach(&graph, &app, "test-graph", &trash);
    io.sync_faults.extend(syncs.iter().cloned());
    let locks = KEYS
        .into_iter()
        .map(|key| (key.into(), Arc::new(Mutex::new(()))))
        .collect();
    let mut host = shared_counters(Host::new(io, locks));
    host.stop();
    for phase in faults {
        host.fs.faults.insert(*phase, [io::ErrorKind::Other].into());
    }
    assert!(matches!(
        host.launch(),
        Disposition::Applied | Disposition::Pending
    ));
    let mut f = Fixture {
        graph,
        app,
        trash,
        host,
        root,
    };
    f.drain();
    f
}

fn fixture() -> Fixture {
    let mut f = Fixture::new();
    f.host
        .register("d.md".into(), "d.md", Arc::new(Mutex::new(())));
    fs::write(f.graph.join("a.md"), OLD).unwrap();
    fs::write(f.graph.join("b.md"), REF).unwrap();
    f
}

fn admit_rename(f: &mut Fixture) {
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
}

/// The admitted rename, durable and applied in memory, then a process death
/// before any page saved, then a cold relaunch.
fn relaunched_after_a_crash_before_the_receiver_published() -> Fixture {
    let mut f = fixture();
    admit_rename(&mut f);
    f.drain();
    assert_eq!(f.host.pages["c.md"].buf.as_deref(), Some(OLD.as_bytes()));
    assert!(!f.graph.join("c.md").exists(), "C is not published");
    let f = reopen(f);
    assert_eq!(f.host.pages["a.md"].buf, None, "A's removal is recovered");
    assert_eq!(f.host.pages["c.md"].buf.as_deref(), Some(OLD.as_bytes()));
    assert_eq!(
        f.host.pages["b.md"].buf.as_deref(),
        Some(REWRITTEN.as_bytes())
    );
    assert_eq!(
        f.host.fs.draft_files(true).len(),
        1,
        "the paired operation draft is immutable: launch does not explode it"
    );
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

/// The draft steps the progress loop runs for settled pages.
fn retire_drafts(f: &mut Fixture) {
    for _ in 0..4 {
        for key in KEYS {
            if f.host.begin_draft_retiring(key) == Disposition::Pending {
                f.drain();
            }
        }
    }
}

fn pending_drafts(f: &Fixture) -> usize {
    f.host.fs.draft_files(true).len()
}

/// Some draft file still holds `text` (custody of the source's text).
fn drafted(f: &Fixture, text: &str) -> bool {
    f.host
        .fs
        .draft_files(true)
        .iter()
        .any(|(_, b)| b.windows(text.len()).any(|w| w == text.as_bytes()))
}

/// Save every held page that will save, receivers first, until none does.
fn settle_all(f: &mut Fixture) {
    for _ in 0..4 {
        for key in ["c.md", "d.md", "b.md", "a.md"] {
            if f.host.start_save(key) == Disposition::Pending {
                for _ in 0..15 {
                    f.host.advance_save();
                    if f.host.job.is_none() {
                        break;
                    }
                }
                assert!(f.host.job.is_none(), "{key}: save did not finish");
            }
        }
    }
    retire_drafts(f);
}

/// A new operation over `key` waits (no chaining for an unresolved pair).
fn blocked(f: &Fixture, key: &str) -> bool {
    f.host.order.touches(&BTreeSet::from([key.to_string()]))
}

/// A rename of A to D, as a probe of whether A's pages take operations.
fn rename_a_to_d(f: &mut Fixture) -> Disposition {
    f.host.rename(
        "a.md",
        "d.md",
        &BTreeSet::new(),
        "A",
        "D",
        tine_core::config::FileNameFormat::TripleLowbar,
    )
}

/// Run `key`'s save to its end, whatever its outcome.
fn run_save(f: &mut Fixture, key: &str) {
    assert_eq!(f.host.start_save(key), Disposition::Pending);
    for _ in 0..15 {
        f.host.advance_save();
        if f.host.job.is_none() {
            return;
        }
    }
    panic!("{key}: save did not finish");
}

/// C and B published, then an outside writer replaces C before A's trash
/// step: the pair is held, A stays.
fn held_before_the_trash_step() -> Fixture {
    let mut f = relaunched_after_a_crash_before_the_receiver_published();
    assert_eq!(f.save("c.md"), Outcome::Published);
    assert_eq!(f.save("b.md"), Outcome::Published);
    fs::write(f.graph.join("c.md"), "- other\n").unwrap();
    run_save(&mut f, "a.md");
    assert_eq!(f.host.start_save("a.md"), Disposition::Disabled, "held");
    assert_eq!(on_disk(&f, "a.md").as_deref(), Some(OLD));
    f
}

fn refused_draft(f: &Fixture, key: &str) -> bool {
    f.host.events.iter().any(
        |e| matches!(e, Event::Refused { page, reason: Refusal::DraftFailed, .. } if page == key),
    )
}

/// Formerly the characterization "a discard of a recovered destination
/// leaves the source only in trash" (HARM: A and C both gone, A's text only
/// in `logseq/.tine-trash/pages`). The host-level Discard of the recovered,
/// absent C is a durable Use Disk: A's removal is cancelled, A keeps its
/// disk file, B still publishes its rewrite (today's cancellation
/// behaviour), and a second relaunch revives nothing.
#[test]
fn a_discard_of_a_recovered_receiver_keeps_the_source_across_relaunches() {
    let mut f = relaunched_after_a_crash_before_the_receiver_published();
    f.send("c.md", RequestKind::Discard);
    assert!(f.host.pages["c.md"].clean(), "C took its disk state");
    assert_eq!(f.host.pages["c.md"].buf, None, "C: no file");
    assert_eq!(
        f.host.pages["a.md"].buf.as_deref(),
        Some(OLD.as_bytes()),
        "A's removal is reverted"
    );
    assert_eq!(pending_drafts(&f), 2, "the operation and its decision");
    assert_eq!(f.host.start_save("a.md"), Disposition::Disabled);
    assert_eq!(f.host.start_save("c.md"), Disposition::Disabled);
    assert_eq!(f.save("b.md"), Outcome::Published);
    retire_drafts(&mut f);
    assert_eq!(on_disk(&f, "a.md").as_deref(), Some(OLD), "A stays");
    assert_eq!(on_disk(&f, "c.md"), None, "C was never written");
    assert_eq!(on_disk(&f, "b.md").as_deref(), Some(REWRITTEN));
    assert!(trashed(&f).is_empty(), "nothing went to trash");
    assert_eq!(pending_drafts(&f), 0, "the operation retired whole");
    // A second relaunch after the choice: nothing to revive.
    let mut f = reopen(f);
    assert!(f.host.pages.is_empty());
    settle_all(&mut f);
    assert_eq!(on_disk(&f, "a.md").as_deref(), Some(OLD));
    assert!(trashed(&f).is_empty());
}

/// Formerly the characterization "a take disk on a recovered destination
/// conflicted by sync leaves the source only in trash". After the relaunch
/// A's removal waits for C's witness (the recovered order), so it cannot
/// publish while C is conflicted; Use Disk on C then cancels the pair.
#[test]
fn a_take_disk_on_a_recovered_receiver_conflicted_by_sync_keeps_the_source() {
    let mut f = relaunched_after_a_crash_before_the_receiver_published();
    fs::write(f.graph.join("c.md"), "- synced\n").unwrap();
    assert_eq!(f.host.observe("c.md"), Disposition::Applied);
    assert!(
        f.host.pages["c.md"].conflict,
        "C conflicts with the delivery"
    );
    assert_eq!(f.host.start_save("c.md"), Disposition::Disabled);
    assert_eq!(
        f.host.start_save("a.md"),
        Disposition::Waiting,
        "A's removal waits for C after the relaunch"
    );
    f.send("c.md", RequestKind::Discard);
    assert_eq!(f.save("b.md"), Outcome::Published);
    retire_drafts(&mut f);
    assert_eq!(on_disk(&f, "a.md").as_deref(), Some(OLD), "A stays");
    assert_eq!(on_disk(&f, "c.md").as_deref(), Some("- synced\n"));
    assert_eq!(on_disk(&f, "b.md").as_deref(), Some(REWRITTEN));
    assert!(trashed(&f).is_empty(), "nothing went to trash");
    assert_eq!(pending_drafts(&f), 0);
    let mut f = reopen(f);
    settle_all(&mut f);
    assert_eq!(on_disk(&f, "a.md").as_deref(), Some(OLD));
    assert_eq!(on_disk(&f, "c.md").as_deref(), Some("- synced\n"));
    assert!(trashed(&f).is_empty());
}

/// A crash right after the acknowledged Use Disk, before any cleanup: the
/// durable decision cancels the pair at the next launch, which recovers
/// neither C's nor A's operation record.
#[test]
fn a_durable_use_disk_is_not_replayed_as_a_removal_after_a_crash() {
    let mut f = relaunched_after_a_crash_before_the_receiver_published();
    f.send("c.md", RequestKind::Discard);
    let mut f = reopen(f);
    assert!(!f.host.pages.contains_key("a.md"), "A is not recovered");
    assert!(!f.host.pages.contains_key("c.md"), "C is not recovered");
    assert_eq!(
        f.host.pages["b.md"].buf.as_deref(),
        Some(REWRITTEN.as_bytes())
    );
    settle_all(&mut f);
    assert_eq!(on_disk(&f, "a.md").as_deref(), Some(OLD));
    assert_eq!(on_disk(&f, "c.md"), None);
    assert_eq!(on_disk(&f, "b.md").as_deref(), Some(REWRITTEN));
    assert!(trashed(&f).is_empty());
    assert_eq!(pending_drafts(&f), 0, "the decision retired after it");
    let mut f = reopen(f);
    assert!(f.host.pages.is_empty());
    settle_all(&mut f);
    assert_eq!(on_disk(&f, "a.md").as_deref(), Some(OLD));
}

/// With no interference, launch finishes the rename: C, then B, then A's
/// removal after both receiver checks; the operation draft retires whole.
#[test]
fn a_recovered_rename_finishes_in_order_after_relaunch() {
    let mut f = relaunched_after_a_crash_before_the_receiver_published();
    assert_eq!(f.host.start_save("a.md"), Disposition::Waiting);
    assert_eq!(f.host.start_save("b.md"), Disposition::Waiting);
    assert_eq!(f.save("c.md"), Outcome::Published);
    assert_eq!(f.host.start_save("a.md"), Disposition::Waiting);
    assert_eq!(f.save("b.md"), Outcome::Published);
    assert_eq!(f.save("a.md"), Outcome::Published);
    retire_drafts(&mut f);
    assert_eq!(on_disk(&f, "a.md"), None);
    assert_eq!(on_disk(&f, "c.md").as_deref(), Some(OLD));
    assert_eq!(on_disk(&f, "b.md").as_deref(), Some(REWRITTEN));
    assert_eq!(trashed(&f), vec![OLD.to_string()]);
    assert_eq!(pending_drafts(&f), 0);
}

/// C published before the process died: launch reinstalls C's record with
/// its creation base, and C's file already holds it. C's save of the same
/// bytes re-witnesses its durability (it is not refused as an outside
/// change), so the recovered order moves on and the rename finishes.
#[test]
fn a_receiver_published_before_the_crash_is_rewitnessed_after_relaunch() {
    let mut f = fixture();
    admit_rename(&mut f);
    f.drain();
    assert_eq!(f.save("c.md"), Outcome::Published);
    let mut f = reopen(f);
    assert_eq!(f.host.pages["c.md"].buf.as_deref(), Some(OLD.as_bytes()));
    assert_eq!(f.host.start_save("b.md"), Disposition::Waiting);
    assert_eq!(f.save("c.md"), Outcome::Published, "re-witnessed");
    assert_eq!(f.save("b.md"), Outcome::Published);
    assert_eq!(f.save("a.md"), Outcome::Published);
    retire_drafts(&mut f);
    assert_eq!(on_disk(&f, "a.md"), None);
    assert_eq!(on_disk(&f, "c.md").as_deref(), Some(OLD));
    assert_eq!(on_disk(&f, "b.md").as_deref(), Some(REWRITTEN));
    assert_eq!(trashed(&f), vec![OLD.to_string()]);
    assert_eq!(pending_drafts(&f), 0);
}

/// The first receiver check: C replaced by an outside writer after its
/// publication and before A's trash step. A's removal is held, A stays.
#[test]
fn a_receiver_replaced_before_the_trash_step_holds_the_removal() {
    let mut f = relaunched_after_a_crash_before_the_receiver_published();
    assert_eq!(f.save("c.md"), Outcome::Published);
    assert_eq!(f.save("b.md"), Outcome::Published);
    fs::write(f.graph.join("c.md"), "- other\n").unwrap();
    assert_eq!(f.host.start_save("a.md"), Disposition::Pending);
    for _ in 0..15 {
        f.host.advance_save();
        if f.host.job.is_none() {
            break;
        }
    }
    assert_eq!(on_disk(&f, "a.md").as_deref(), Some(OLD), "A stays");
    assert!(trashed(&f).is_empty());
    assert_eq!(f.host.start_save("a.md"), Disposition::Disabled, "held");
    retire_drafts(&mut f);
    assert!(drafted(&f, OLD), "the operation draft is retained");
    assert!(blocked(&f, "a.md") && blocked(&f, "b.md") && blocked(&f, "c.md"));
    // Another program's exact bytes (a new file) certify nothing: the pair
    // stays held. (This once rewrote C in place, which re-armed the pair
    // only when the rewrite kept C's size and timestamp tick, so the own
    // publication's stamp still matched: a flaky pass.)
    replace_outside(&f, "c.md", OLD);
    assert_eq!(f.host.observe("c.md"), Disposition::Applied);
    assert_eq!(f.host.start_save("a.md"), Disposition::Disabled, "held");
    // The receiver exact again by this host's own publication re-arms the
    // pair; the removal then finishes.
    replace_outside(&f, "c.md", "- other\n");
    assert_eq!(f.host.observe("c.md"), Disposition::Applied);
    f.edit("c.md", OLD);
    assert_eq!(f.save("c.md"), Outcome::Published);
    assert_eq!(f.save("a.md"), Outcome::Published);
    assert!(
        !blocked(&f, "a.md") && !blocked(&f, "b.md") && !blocked(&f, "c.md"),
        "the settled pair releases its pages before its draft retires"
    );
    retire_drafts(&mut f);
    assert_eq!(trashed(&f), vec![OLD.to_string()]);
    assert_eq!(pending_drafts(&f), 0);
}

/// The second receiver check: C replaced between A's trash move and the
/// end of A's durability steps. The pair stays pending and its draft keeps
/// A's text, also across a relaunch.
#[test]
fn a_receiver_replaced_between_the_checks_retains_the_source_text() {
    let mut f = relaunched_after_a_crash_before_the_receiver_published();
    assert_eq!(f.save("c.md"), Outcome::Published);
    assert_eq!(f.save("b.md"), Outcome::Published);
    assert_eq!(f.host.start_save("a.md"), Disposition::Pending);
    while f.graph.join("a.md").exists() {
        f.host.advance_save();
        assert!(f.host.job.is_some());
    }
    fs::write(f.graph.join("c.md"), "- other\n").unwrap();
    for _ in 0..15 {
        f.host.advance_save();
        if f.host.job.is_none() {
            break;
        }
    }
    assert_eq!(trashed(&f), vec![OLD.to_string()]);
    retire_drafts(&mut f);
    assert!(drafted(&f, OLD), "the draft keeps A's text as custody");
    let mut f = reopen(f);
    settle_all(&mut f);
    assert!(drafted(&f, OLD), "and keeps it after a relaunch");
    assert_eq!(on_disk(&f, "c.md").as_deref(), Some("- other\n"));
}

/// Two pairs in one operation (A to C, B to D): Use Disk on C after a
/// relaunch cancels that pair only; B's removal still finishes.
#[test]
fn a_cancelled_pair_leaves_its_sibling_pair_able_to_finish() {
    let mut f = fixture();
    let effects = BTreeMap::from([
        ("a.md".into(), (None, bytes(OLD))),
        ("c.md".into(), (bytes(OLD), None)),
        ("b.md".into(), (None, bytes(REF))),
        ("d.md".into(), (bytes(REF), None)),
    ]);
    let moves = [
        ("c.md".to_string(), "a.md".to_string()),
        ("d.md".into(), "b.md".into()),
    ];
    assert_eq!(f.host.apply(&effects, &moves), Disposition::Pending);
    f.drain();
    let mut f = reopen(f);
    f.send("c.md", RequestKind::Discard);
    assert_eq!(f.host.pages["a.md"].buf.as_deref(), Some(OLD.as_bytes()));
    settle_all(&mut f);
    assert_eq!(on_disk(&f, "a.md").as_deref(), Some(OLD), "A stays");
    assert_eq!(on_disk(&f, "c.md"), None);
    assert_eq!(on_disk(&f, "b.md"), None, "B moved");
    assert_eq!(on_disk(&f, "d.md").as_deref(), Some(REF));
    assert_eq!(trashed(&f), vec![REF.to_string()]);
    assert_eq!(pending_drafts(&f), 0);
}

/// A legacy operation draft (the nonempty-vector body) recovers exactly as
/// before: exploded into page drafts, with no recovered order.
#[test]
fn a_legacy_operation_draft_recovers_as_before() {
    let mut f = fixture();
    let records = vec![
        Record {
            page: "a.md".into(),
            wseq: 1,
            version: 1,
            base: Base::Known(bytes(OLD)),
            bytes: None,
        },
        Record {
            page: "c.md".into(),
            wseq: 2,
            version: 2,
            base: Base::Known(None),
            bytes: bytes(OLD),
        },
    ];
    let mut vehicle = Vehicle::write(drafts::op_name(), &records);
    while !matches!(vehicle.stage, Stage::Present | Stage::Absent) {
        vehicle.advance(&mut f.host.fs);
    }
    assert_eq!(vehicle.stage, Stage::Present);
    let mut f = reopen(f);
    assert_eq!(f.host.pages["a.md"].buf, None);
    assert_eq!(
        f.host.start_save("a.md"),
        Disposition::Pending,
        "no recovered order gates a legacy draft"
    );
    let names: Vec<_> = f
        .host
        .fs
        .draft_files(true)
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert!(names.iter().all(|n| n.starts_with("p-")), "{names:?}");
}

/// An unreadable decision matching the operation blocks its destructive
/// step, stays in place (never quarantined), and surfaces a draft error.
#[test]
fn an_unreadable_decision_blocks_the_removal_and_stays_in_place() {
    let f = relaunched_after_a_crash_before_the_receiver_published();
    let op = f.host.fs.draft_files(true)[0].0.clone();
    let id = drafts::op_id(&op).unwrap();
    let name = drafts::decision_vehicle("c.md", &id, 2);
    let mut f = f;
    let mut vehicle = Vehicle::write(name.clone(), &[]);
    vehicle.bytes = Some(b"TINEDRF2\x02 not a decision, nor a checksum".to_vec());
    while !matches!(vehicle.stage, Stage::Present | Stage::Absent) {
        vehicle.advance(&mut f.host.fs);
    }
    let mut f = reopen(f);
    assert!(f
        .host
        .events
        .iter()
        .any(|e| matches!(e, Event::DraftError { effect, .. } if *effect == name)));
    assert_eq!(
        f.save("c.md"),
        Outcome::Published,
        "the receiver still saves"
    );
    assert_eq!(f.save("b.md"), Outcome::Published);
    assert_eq!(f.host.start_save("a.md"), Disposition::Disabled, "blocked");
    retire_drafts(&mut f);
    assert_eq!(on_disk(&f, "a.md").as_deref(), Some(OLD));
    let names: Vec<_> = f
        .host
        .fs
        .draft_files(true)
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert!(names.contains(&name) && names.contains(&op), "{names:?}");
}

/// A decision that cannot be written refuses the Use Disk: nothing is
/// acknowledged, and A's removal still waits for C.
#[test]
fn a_failed_decision_write_refuses_the_use_disk() {
    let mut f = relaunched_after_a_crash_before_the_receiver_published();
    f.host
        .fs
        .faults
        .insert(Phase::DraftTemp, [io::ErrorKind::Other].into());
    f.send("c.md", RequestKind::Discard);
    assert!(refused_draft(&f, "c.md"));
    assert_eq!(f.host.pages["c.md"].buf.as_deref(), Some(OLD.as_bytes()));
    assert_eq!(f.host.pages["a.md"].buf, None);
    assert_eq!(f.host.start_save("a.md"), Disposition::Waiting);
    f.host.fs.faults.clear();
    f.send("c.md", RequestKind::Discard);
    assert_eq!(f.host.pages["a.md"].buf.as_deref(), Some(OLD.as_bytes()));
}

/// One primitive step of the whole rename, from admission through cleanup:
/// a draft phase, a save phase, a save start (receivers first) or a draft
/// retirement. False once nothing is left to do.
fn step(f: &mut Fixture) -> bool {
    if f.host.worker.is_some() {
        f.host.advance_draft();
        return true;
    }
    if f.host.job.is_some() {
        f.host.advance_save();
        return true;
    }
    for key in ["c.md", "b.md", "a.md"] {
        if f.host.start_save(key) == Disposition::Pending {
            return true;
        }
    }
    for key in ["a.md", "b.md", "c.md"] {
        if f.host.begin_draft_retiring(key) == Disposition::Pending {
            return true;
        }
    }
    false
}

/// The end states a rename may reach: not done, or done.
fn untouched_or_renamed(f: &Fixture, cut: usize) {
    let state = (on_disk(f, "a.md"), on_disk(f, "c.md"), on_disk(f, "b.md"));
    let untouched = (Some(OLD.into()), None, Some(REF.into()));
    let renamed = (None, Some(OLD.into()), Some(REWRITTEN.into()));
    assert!(
        state == untouched || state == renamed,
        "cut {cut}: {state:?}"
    );
    if state == renamed {
        assert_eq!(trashed(f), vec![OLD.to_string()], "cut {cut}");
    } else {
        assert!(trashed(f).is_empty(), "cut {cut}");
    }
    assert_eq!(pending_drafts(f), 0, "cut {cut}");
}

/// A process death at every step of the rename (draft temp, rename and
/// directory sync; receiver and referrer temp, guard, rename and directory
/// sync; source guard and receiver check, marker, trash move, trash sync,
/// source-directory sync and second check; each cleanup unlink and sync),
/// then a cold relaunch that runs to completion: the rename is either not
/// done or done, A's text is never only in trash, and nothing is left.
#[test]
fn a_rename_cut_at_every_step_ends_untouched_or_renamed() {
    let mut total = 0;
    {
        let mut f = fixture();
        admit_rename(&mut f);
        while step(&mut f) {
            total += 1;
            assert!(total < 200);
        }
        untouched_or_renamed(&f, total);
    }
    assert!(total > 20, "{total} steps");
    for cut in 0..total {
        let mut f = fixture();
        admit_rename(&mut f);
        for _ in 0..cut {
            step(&mut f);
        }
        let mut f = reopen(f);
        // A's text is somewhere active or in the paired draft at the cut.
        assert!(
            on_disk(&f, "a.md").as_deref() == Some(OLD)
                || on_disk(&f, "c.md").as_deref() == Some(OLD)
                || drafted(&f, OLD),
            "cut {cut}"
        );
        let mut steps = 0;
        while step(&mut f) {
            steps += 1;
            assert!(steps < 200, "cut {cut}: recovery did not settle");
        }
        untouched_or_renamed(&f, cut);
    }
}

/// The same cuts with the user's Use Disk on the recovered receiver right
/// after the relaunch: A is never removed.
#[test]
fn a_use_disk_after_a_cut_never_removes_the_source() {
    let mut total = 0;
    {
        let mut f = fixture();
        admit_rename(&mut f);
        while step(&mut f) {
            total += 1;
        }
    }
    for cut in 0..total {
        let mut f = fixture();
        admit_rename(&mut f);
        for _ in 0..cut {
            step(&mut f);
        }
        let mut f = reopen(f);
        let receiver_pending = f
            .host
            .pages
            .get("c.md")
            .is_some_and(|p| p.risk && !f.graph.join("c.md").exists());
        if !receiver_pending {
            continue;
        }
        f.send("c.md", RequestKind::Discard);
        let mut f = reopen(f);
        while step(&mut f) {}
        assert_eq!(on_disk(&f, "a.md").as_deref(), Some(OLD), "cut {cut}");
        assert!(trashed(&f).is_empty(), "cut {cut}");
        assert_eq!(pending_drafts(&f), 0, "cut {cut}");
    }
}

/// A held pair blocks new operations on its pages until it is resolved. Use
/// Disk on its receiver resolves it without a relaunch: the cancellation is
/// durable first, the source keeps its disk file, the pages take operations
/// again and the draft retires; no later launch replays the removal.
#[test]
fn a_use_disk_on_a_held_receiver_cancels_the_pair_and_releases_its_pages() {
    let mut f = held_before_the_trash_step();
    assert!(blocked(&f, "a.md") && blocked(&f, "b.md") && blocked(&f, "c.md"));
    assert_eq!(rename_a_to_d(&mut f), Disposition::Waiting);
    f.send("c.md", RequestKind::Discard);
    assert!(
        !blocked(&f, "a.md") && !blocked(&f, "b.md") && !blocked(&f, "c.md"),
        "released at once, before the draft retires"
    );
    assert_eq!(
        f.host.pages["a.md"].buf.as_deref(),
        Some(OLD.as_bytes()),
        "A's removal is reverted"
    );
    assert_eq!(f.host.pages["c.md"].buf.as_deref(), Some(&b"- other\n"[..]));
    retire_drafts(&mut f);
    assert_eq!(
        pending_drafts(&f),
        0,
        "the operation and its decision retired"
    );
    assert_eq!(on_disk(&f, "a.md").as_deref(), Some(OLD), "A stays");
    assert_eq!(on_disk(&f, "c.md").as_deref(), Some("- other\n"));
    assert!(trashed(&f).is_empty());
    let mut f = reopen(f);
    assert!(f.host.pages.is_empty(), "nothing revives");
    settle_all(&mut f);
    assert_eq!(on_disk(&f, "a.md").as_deref(), Some(OLD));
    assert!(trashed(&f).is_empty());
}

/// A crash right after that Use Disk, before any cleanup: the durable
/// decision keeps the pair cancelled at the next launch.
#[test]
fn a_use_disk_on_a_held_receiver_survives_a_crash() {
    let mut f = held_before_the_trash_step();
    f.send("c.md", RequestKind::Discard);
    let mut f = reopen(f);
    assert!(!f.host.pages.contains_key("a.md"), "A is not recovered");
    assert!(!blocked(&f, "a.md"));
    settle_all(&mut f);
    assert_eq!(on_disk(&f, "a.md").as_deref(), Some(OLD));
    assert_eq!(on_disk(&f, "c.md").as_deref(), Some("- other\n"));
    assert!(trashed(&f).is_empty());
    assert_eq!(pending_drafts(&f), 0);
}

/// A launch whose draft store could not sync blocks every paired removal.
/// A successful Retry of the draft store resumes it without a relaunch: A's
/// removal then finishes and the pages are released.
#[test]
fn a_successful_draft_store_retry_releases_a_blocked_pair() {
    let mut f = fixture();
    admit_rename(&mut f);
    f.drain();
    let mut f = reopen_with(f, &[Phase::DraftSync]);
    assert!(f.host.fs.draft_status().unavailable.is_some());
    assert_eq!(f.save("c.md"), Outcome::Published);
    assert_eq!(f.save("b.md"), Outcome::Published);
    assert_eq!(f.host.start_save("a.md"), Disposition::Disabled, "blocked");
    assert!(blocked(&f, "a.md"));
    assert!(matches!(
        f.host.drafts_retry(),
        Ok(Disposition::Applied | Disposition::Pending)
    ));
    f.drain();
    assert_eq!(f.save("a.md"), Outcome::Published);
    assert!(!blocked(&f, "a.md") && !blocked(&f, "c.md"));
    retire_drafts(&mut f);
    assert_eq!(on_disk(&f, "a.md"), None);
    assert_eq!(on_disk(&f, "c.md").as_deref(), Some(OLD));
    assert_eq!(trashed(&f), vec![OLD.to_string()]);
    assert_eq!(pending_drafts(&f), 0);
}

/// A pair an unreadable decision blocks: Use Disk on its receiver writes a
/// readable cancellation, which releases the pages; the operation retires
/// with both decisions, and A stays.
#[test]
fn a_use_disk_on_a_blocked_receiver_releases_its_pages() {
    let f = relaunched_after_a_crash_before_the_receiver_published();
    let op = f.host.fs.draft_files(true)[0].0.clone();
    let id = drafts::op_id(&op).unwrap();
    let name = drafts::decision_vehicle("c.md", &id, 2);
    let mut f = f;
    let mut vehicle = Vehicle::write(name, &[]);
    vehicle.bytes = Some(b"TINEDRF2\x02 not a decision, nor a checksum".to_vec());
    while !matches!(vehicle.stage, Stage::Present | Stage::Absent) {
        vehicle.advance(&mut f.host.fs);
    }
    let mut f = reopen(f);
    assert_eq!(f.save("c.md"), Outcome::Published);
    assert_eq!(f.save("b.md"), Outcome::Published);
    assert_eq!(f.host.start_save("a.md"), Disposition::Disabled, "blocked");
    assert!(blocked(&f, "a.md"));
    f.send("c.md", RequestKind::Discard);
    assert!(!blocked(&f, "a.md") && !blocked(&f, "c.md"));
    assert_eq!(f.host.pages["a.md"].buf.as_deref(), Some(OLD.as_bytes()));
    retire_drafts(&mut f);
    assert_eq!(pending_drafts(&f), 0, "the operation and both decisions");
    assert_eq!(on_disk(&f, "a.md").as_deref(), Some(OLD));
    assert_eq!(on_disk(&f, "c.md").as_deref(), Some(OLD));
    assert!(trashed(&f).is_empty());
}

/// Newer input typed into the recovered source supersedes its removal: its
/// save publishes the text, the pair is disarmed, the pages are released and
/// the draft retires; no later launch replays the removal.
#[test]
fn newer_source_text_disarms_the_pair_and_releases_its_pages() {
    let mut f = relaunched_after_a_crash_before_the_receiver_published();
    assert_eq!(f.save("c.md"), Outcome::Published);
    assert_eq!(f.save("b.md"), Outcome::Published);
    f.edit("a.md", "- kept\n");
    assert!(blocked(&f, "a.md"));
    assert_eq!(f.save("a.md"), Outcome::Published);
    assert!(!blocked(&f, "a.md") && !blocked(&f, "c.md"));
    retire_drafts(&mut f);
    assert_eq!(pending_drafts(&f), 0);
    assert_eq!(on_disk(&f, "a.md").as_deref(), Some("- kept\n"));
    assert_eq!(on_disk(&f, "c.md").as_deref(), Some(OLD));
    assert!(trashed(&f).is_empty());
    let mut f = reopen(f);
    settle_all(&mut f);
    assert_eq!(on_disk(&f, "a.md").as_deref(), Some("- kept\n"));
    assert!(trashed(&f).is_empty());
}

/// Another program replaces `key` with `text` (a new file at the path, as
/// a sync client delivers it): equal bytes carry no witness of this host.
fn replace_outside(f: &Fixture, key: &str, text: &str) {
    let temp = f.graph.join(format!(".{key}.outside"));
    fs::write(&temp, text).unwrap();
    fs::rename(&temp, f.graph.join(key)).unwrap();
}

/// The review's launch trace. A moved to trash, then C replaced before the
/// post-check: the pair is held with A removed. Use Disk on C records the
/// cancellation (Restore: A's slot is free and its text is only in the
/// trash and the operation draft). An outside program then writes C's
/// image back, and the app relaunches. No witness survives the relaunch, so
/// the byte match certifies nothing: the pair stays Restore and the
/// operation draft keeps A's admitted text, across a second relaunch too.
/// Before, launch certified C on bytes alone, cancelled the pair and
/// retired the draft, leaving A's text only in the trash.
#[test]
fn a_launch_time_byte_match_keeps_the_source_text_until_restored() {
    let mut f = relaunched_after_a_crash_before_the_receiver_published();
    assert_eq!(f.save("c.md"), Outcome::Published);
    assert_eq!(f.save("b.md"), Outcome::Published);
    assert_eq!(f.host.start_save("a.md"), Disposition::Pending);
    while f.graph.join("a.md").exists() {
        f.host.advance_save();
        assert!(f.host.job.is_some());
    }
    fs::write(f.graph.join("c.md"), "- other\n").unwrap();
    for _ in 0..15 {
        f.host.advance_save();
        if f.host.job.is_none() {
            break;
        }
    }
    assert_eq!(trashed(&f), vec![OLD.to_string()]);
    f.send("c.md", RequestKind::Discard);
    assert!(f.host.order.restorable("a.md").is_some(), "Restore");
    replace_outside(&f, "c.md", OLD);
    let mut f = reopen(f);
    assert!(
        f.host.order.restorable("a.md").is_some(),
        "a launch-time byte match does not cancel the pair"
    );
    assert!(blocked(&f, "a.md") && blocked(&f, "c.md"));
    settle_all(&mut f);
    assert!(drafted(&f, OLD), "the operation draft keeps A's text");
    let mut f = reopen(f);
    settle_all(&mut f);
    assert!(drafted(&f, OLD), "and keeps it after a second relaunch");
    assert!(f.host.order.restorable("a.md").is_some());
    assert_eq!(on_disk(&f, "a.md"), None);
}

/// The running-session trace: C was published by this host (witnessed),
/// then another program wrote the same bytes as a new file. Use Disk on C
/// now finds C not certified, as the removal's pre-check does, so it writes
/// the cancellation (the model's `partner`) and A's removal is reverted.
/// Before, Use Disk compared bytes only, wrote no decision, and the
/// pre-check then held the pair.
#[test]
fn a_use_disk_on_a_matching_receiver_without_a_witness_cancels_the_pair() {
    let mut f = relaunched_after_a_crash_before_the_receiver_published();
    assert_eq!(f.save("c.md"), Outcome::Published);
    assert_eq!(f.save("b.md"), Outcome::Published);
    replace_outside(&f, "c.md", OLD);
    f.send("c.md", RequestKind::Discard);
    assert_eq!(pending_drafts(&f), 2, "the operation and its decision");
    assert_eq!(
        f.host.pages["a.md"].buf.as_deref(),
        Some(OLD.as_bytes()),
        "A's removal is reverted"
    );
    assert_eq!(f.host.start_save("a.md"), Disposition::Disabled);
    retire_drafts(&mut f);
    assert_eq!(pending_drafts(&f), 0);
    assert_eq!(on_disk(&f, "a.md").as_deref(), Some(OLD), "A stays");
    assert_eq!(on_disk(&f, "c.md").as_deref(), Some(OLD));
    assert!(trashed(&f).is_empty());
    let mut f = reopen(f);
    settle_all(&mut f);
    assert_eq!(on_disk(&f, "a.md").as_deref(), Some(OLD));
    assert!(trashed(&f).is_empty());
}

/// PC-9: the adapter keeps a durability witness only for the receiver of an
/// unresolved pair. A plain page's publication keeps none; the receiver's
/// is kept while its pair owes A's removal and released once it settles;
/// a closed page releases its own.
#[test]
fn durability_witnesses_are_kept_only_for_unresolved_receivers() {
    let mut f = relaunched_after_a_crash_before_the_receiver_published();
    assert!(f.host.fs.witnessed().is_empty(), "none survives a relaunch");
    assert_eq!(f.save("c.md"), Outcome::Published);
    assert_eq!(f.host.fs.witnessed(), vec!["c.md"], "the receiver's");
    assert_eq!(f.save("b.md"), Outcome::Published);
    assert_eq!(f.host.fs.witnessed(), vec!["c.md"], "no referrer's");
    assert_eq!(f.save("a.md"), Outcome::Published);
    assert!(f.host.fs.witnessed().is_empty(), "released at settlement");
    retire_drafts(&mut f);
    assert_eq!(on_disk(&f, "c.md").as_deref(), Some(OLD));
    // A plain edit and save, then the page closes.
    f.edit("d.md", "- d\n");
    assert_eq!(f.save("d.md"), Outcome::Published);
    assert!(f.host.fs.witnessed().is_empty(), "a plain page keeps none");
}

/// PC-9 with a closed receiver: a receiver whose pair still owes its
/// source's removal keeps its witness while closed, so the rename still
/// finishes; the witness leaves at settlement.
#[test]
fn a_closed_receiver_keeps_its_witness_while_its_pair_owes() {
    let mut f = relaunched_after_a_crash_before_the_receiver_published();
    assert_eq!(f.save("c.md"), Outcome::Published);
    let keys = BTreeSet::from(["c.md".to_string()]);
    f.host.with_locks(&keys, |h| h.close_clean("c.md"));
    assert!(!f.host.pages.contains_key("c.md"), "C closed");
    assert_eq!(
        f.host.fs.witnessed(),
        vec!["c.md"],
        "the pair still needs it"
    );
    assert_eq!(f.save("b.md"), Outcome::Published);
    assert_eq!(f.save("a.md"), Outcome::Published, "C is still certified");
    assert_eq!(trashed(&f), vec![OLD.to_string()]);
    assert!(f.host.fs.witnessed().is_empty());
}

/// The source `src` (A's text) renamed to C over B; C's and B's saves
/// publish, A's removal publishes, then another program replaces C before
/// the post-check and Use Disk on C records the cancellation: Restore (A's
/// slot is free, its text is in the trash and the operation draft). The
/// user's Restore is admitted; its save has not started.
fn restoring(src: &str) -> Fixture {
    let mut f = fixture();
    if src != "a.md" {
        fs::create_dir_all(f.graph.join(src).parent().unwrap()).unwrap();
        fs::write(f.graph.join(src), OLD).unwrap();
        f.host.register(src.into(), src, Arc::new(Mutex::new(())));
    }
    assert_eq!(
        f.host.rename(
            src,
            "c.md",
            &BTreeSet::from(["b.md".into()]),
            "A",
            "C",
            tine_core::config::FileNameFormat::TripleLowbar
        ),
        Disposition::Pending
    );
    f.drain();
    assert_eq!(f.save("c.md"), Outcome::Published);
    assert_eq!(f.save("b.md"), Outcome::Published);
    assert_eq!(f.host.start_save(src), Disposition::Pending);
    while f.graph.join(src).exists() {
        f.host.advance_save();
        assert!(f.host.job.is_some());
    }
    fs::write(f.graph.join("c.md"), "- other\n").unwrap();
    for _ in 0..15 {
        f.host.advance_save();
        if f.host.job.is_none() {
            break;
        }
    }
    assert_eq!(trashed(&f), vec![OLD.to_string()]);
    f.send("c.md", RequestKind::Discard);
    assert!(f.host.order.restorable(src).is_some(), "Restore");
    assert_eq!(f.host.restore_source(src), Disposition::Applied);
    f
}

/// Run the restoring save of `src` until its publication's rename landed,
/// then stop stepping (a process death before its directory sync).
fn cut_before_the_directory_sync(f: &mut Fixture, src: &str) {
    assert_eq!(f.host.start_save(src), Disposition::Pending);
    while !f.graph.join(src).exists() {
        f.host.advance_save();
        assert!(f.host.job.is_some());
    }
    assert_eq!(f.host.job.as_ref().unwrap().phase, SavePhase::DirectorySync);
}

/// Custody second cut (regression `launch-custody-second-cut`): a crash
/// after the restoring publication's rename and before its directory sync,
/// then a relaunch whose source-directory sync fails while the draft store
/// still retires files. A's bytes are its preimage, but nothing witnesses
/// that file: a power cut could still drop it. The pair stays Retained and
/// the operation draft keeps A's text. Before, launch compared bytes only,
/// cancelled the pair and retired the draft. A relaunch whose sync
/// succeeds cancels the pair, as before.
#[test]
fn a_relaunch_whose_source_directory_sync_fails_keeps_the_original() {
    let mut f = restoring("a.md");
    cut_before_the_directory_sync(&mut f, "a.md");
    let root = f.graph.clone();
    let mut f = reopen_syncing(f, &[], &[(root, io::ErrorKind::Other)]);
    assert_eq!(on_disk(&f, "a.md").as_deref(), Some(OLD));
    assert!(
        f.host.order.restorable("a.md").is_some(),
        "an unwitnessed byte match keeps custody"
    );
    assert!(blocked(&f, "a.md"));
    retire_drafts(&mut f);
    assert!(drafted(&f, OLD), "the operation draft keeps A's text");
    // The success path: every sync succeeds, the pair cancels as today.
    let mut f = reopen(f);
    assert!(f.host.order.restorable("a.md").is_none(), "cancelled");
    assert!(!blocked(&f, "a.md"));
    settle_all(&mut f);
    assert!(!drafted(&f, OLD), "custody released once witnessed");
    assert_eq!(on_disk(&f, "a.md").as_deref(), Some(OLD));
}

/// The nested source: the graph root's sync keeps failing, and the
/// restoring save of `sub/a.md` publishes (its own directory sync covers
/// only `sub`). That save pays no ancestor's debt, so the restoration does
/// not cancel the pair: custody holds, across a relaunch that still cannot
/// sync the root, until a launch syncs it. Before, the publication's byte
/// match cancelled the pair.
#[test]
fn a_nested_restoration_keeps_custody_until_the_graph_root_syncs() {
    let mut f = restoring("sub/a.md");
    let root = f.graph.clone();
    f.host
        .fs
        .sync_faults
        .insert(root.clone(), io::ErrorKind::Other);
    assert_eq!(f.save("sub/a.md"), Outcome::Published, "sub synced");
    assert_eq!(on_disk(&f, "sub/a.md").as_deref(), Some(OLD));
    assert!(
        f.host.order.restorable("sub/a.md").is_some(),
        "the root's entry is not witnessed"
    );
    retire_drafts(&mut f);
    assert!(drafted(&f, OLD));
    let mut f = reopen_syncing(f, &[], &[(root, io::ErrorKind::Other)]);
    assert!(f.host.order.restorable("sub/a.md").is_some());
    retire_drafts(&mut f);
    assert!(drafted(&f, OLD), "still held");
    let mut f = reopen(f);
    assert!(f.host.order.restorable("sub/a.md").is_none(), "cancelled");
    settle_all(&mut f);
    assert!(!drafted(&f, OLD));
}

/// Use Disk on C while A still has its original file, whose directory
/// sync fails: the decision cannot anchor A's text on its bytes, so the
/// pair is retained and the draft keeps it. Before, Use Disk cancelled on
/// the byte match. The next launch that syncs A's directory cancels.
#[test]
fn a_use_disk_whose_source_directory_sync_fails_keeps_custody() {
    let mut f = relaunched_after_a_crash_before_the_receiver_published();
    assert_eq!(f.save("c.md"), Outcome::Published);
    assert_eq!(f.save("b.md"), Outcome::Published);
    replace_outside(&f, "c.md", OLD);
    let root = f.graph.clone();
    f.host.fs.sync_faults.insert(root, io::ErrorKind::Other);
    f.send("c.md", RequestKind::Discard);
    assert_eq!(
        f.host.pages["a.md"].buf.as_deref(),
        Some(OLD.as_bytes()),
        "A's removal is reverted"
    );
    assert!(f.host.order.restorable("a.md").is_some(), "retained");
    retire_drafts(&mut f);
    assert!(drafted(&f, OLD));
    let mut f = reopen(f);
    assert!(f.host.order.restorable("a.md").is_none());
    settle_all(&mut f);
    assert!(!drafted(&f, OLD));
    assert_eq!(on_disk(&f, "a.md").as_deref(), Some(OLD));
    assert!(trashed(&f).is_empty());
}

/// A graph whose filesystem offers no directory sync (NFS, FUSE): the
/// source gets the same weak witness the save path and the receiver get
/// there (a decision the fix-launch-custody receipt hands back). The
/// decision still syncs the chain, and a failure would keep custody; an
/// Unsupported answer does not keep it forever.
#[test]
fn an_unsupported_directory_sync_gets_the_weak_witness() {
    let mut f = restoring("a.md");
    cut_before_the_directory_sync(&mut f, "a.md");
    let root = f.graph.clone();
    let mut f = reopen_syncing(f, &[], &[(root, io::ErrorKind::Unsupported)]);
    assert!(f.host.order.restorable("a.md").is_none(), "cancelled");
    settle_all(&mut f);
    assert!(!drafted(&f, OLD));
    assert_eq!(on_disk(&f, "a.md").as_deref(), Some(OLD));
}
