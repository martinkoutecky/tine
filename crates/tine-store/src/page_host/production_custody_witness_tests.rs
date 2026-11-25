//! The custody source witness against develop's settlement table (model
//! v3): a source anchors its pair's original text with matching bytes and
//! a durable (D) or weak (W) witness; a failed witness step (U) takes the
//! develop outcome of each cell's non-matching family, keeping the draft.
//! Each test names its cell (E1 launch, E2/E3 Use Disk on the source/the
//! receiver, E4/E5 the source's publication of newer text/of a restoration,
//! E7 the removal checks).
use super::*;
use crate::page_host::io::{SourceWitness, WitnessStep};
use crate::page_host::order::{PairChoice, PairPhase};
use crate::page_host::production::{WITNESS_HOOK, WITNESS_READS};

fn state(f: &Fixture, source: &str) -> (PairPhase, PairChoice) {
    f.host
        .order
        .source_state(source)
        .expect("a pair with that source")
}

/// `storage_diag`'s failed source witnesses at `step`.
fn witness_failures(f: &Fixture, step: &str) -> u64 {
    f.host.fs.storage_diag.snapshot()["custodySourceWitnessFailed"][step]
        .as_u64()
        .unwrap()
}

/// Every sync of the graph root fails (`EIO`-like) until healed.
fn fail_root(f: &mut Fixture) {
    let root = f.graph.clone();
    f.host.fs.sync_faults.insert(root, io::ErrorKind::Other);
}

fn heal_root(f: &mut Fixture) {
    let root = f.graph.clone();
    f.host.fs.sync_faults.remove(&root);
}

/// Another program acts just before the witness's `step`.
fn before_witness_step(step: WitnessStep, act: impl FnOnce() + 'static) {
    WITNESS_HOOK.with(|hook| *hook.borrow_mut() = Some((step, Box::new(act))));
}

fn refused(f: &Fixture, key: &str, why: Refusal) -> bool {
    f.host
        .events
        .iter()
        .any(|e| matches!(e, Event::Refused { page, reason, .. } if page == key && *reason == why))
}

/// Make `key` unreadable: a directory stands at its path.
fn unreadable(f: &Fixture, key: &str) {
    let path = f.graph.join(key);
    let _ = fs::remove_file(&path);
    fs::create_dir(&path).unwrap();
}

/// The rename of the nested source `sub/a.md` to C over B, C and B
/// published: the pair is Armed. A save of `sub/a.md` syncs only `sub`.
fn nested_pair() -> Fixture {
    let mut f = fixture();
    fs::create_dir_all(f.graph.join("sub")).unwrap();
    fs::write(f.graph.join("sub/a.md"), OLD).unwrap();
    f.host
        .register("sub/a.md".into(), "sub/a.md", Arc::new(Mutex::new(())));
    assert_eq!(
        f.host.rename(
            "sub/a.md",
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
    assert_eq!(state(&f, "sub/a.md").0, PairPhase::Armed);
    f
}

/// E2·U·C (the first witness cut retained it): Use Disk on the source whose
/// witness fails reads the receiver, as develop does for a non-matching
/// source, and a certified receiver still cancels. One diagnostics record
/// names the failing step.
#[test]
fn a_use_disk_on_the_source_with_a_failed_witness_cancels_on_a_certified_receiver() {
    let mut f = relaunched_after_a_crash_before_the_receiver_published();
    assert_eq!(f.save("c.md"), Outcome::Published);
    assert_eq!(f.save("b.md"), Outcome::Published);
    fail_root(&mut f);
    f.send("a.md", RequestKind::Discard);
    assert_eq!(
        state(&f, "a.md"),
        (PairPhase::Cancelled, PairChoice::SourceDisk)
    );
    assert_eq!(witness_failures(&f, "directorySync"), 1);
    assert!(!blocked(&f, "a.md"));
}

/// E2·U·X (review R1-B2): the failed witness with an unreadable receiver
/// records the choice as Retained (the draft keeps A's text) instead of
/// refusing it.
#[test]
fn a_use_disk_on_the_source_with_a_failed_witness_and_an_unreadable_receiver_is_retained() {
    let mut f = relaunched_after_a_crash_before_the_receiver_published();
    fail_root(&mut f);
    unreadable(&f, "c.md");
    f.send("a.md", RequestKind::Discard);
    assert!(!refused(&f, "a.md", Refusal::ReadFailed));
    assert_eq!(
        state(&f, "a.md"),
        (PairPhase::Retained, PairChoice::SourceDisk)
    );
    assert_eq!(pending_drafts(&f), 2, "the operation and its decision");
    retire_drafts(&mut f);
    assert!(drafted(&f, OLD));
}

/// E2·O·X stays develop's: a source with other bytes and an unreadable
/// receiver refuses Use Disk (no choice recorded).
#[test]
fn a_use_disk_on_a_source_with_other_bytes_and_an_unreadable_receiver_is_refused() {
    let mut f = relaunched_after_a_crash_before_the_receiver_published();
    fs::write(f.graph.join("a.md"), "- edited\n").unwrap();
    unreadable(&f, "c.md");
    f.send("a.md", RequestKind::Discard);
    assert!(refused(&f, "a.md", Refusal::ReadFailed));
    assert_eq!(state(&f, "a.md").1, PairChoice::None);
}

/// E2·D: a durable witness cancels without reading the receiver (develop's
/// byte match), even when the receiver is unreadable.
#[test]
fn a_use_disk_on_a_durably_witnessed_source_needs_no_receiver_read() {
    let mut f = relaunched_after_a_crash_before_the_receiver_published();
    unreadable(&f, "c.md");
    f.send("a.md", RequestKind::Discard);
    assert_eq!(
        state(&f, "a.md"),
        (PairPhase::Cancelled, PairChoice::SourceDisk)
    );
}

/// E5·U (head retained it): the restoration's publication whose witness
/// fails (the graph root's sync; the save itself synced only `sub`) leaves
/// the pair awaiting its restoration, as develop does for other bytes, and
/// the draft keeps A's text. A relaunch whose syncs succeed witnesses the
/// restored file and cancels.
#[test]
fn a_restoration_whose_witness_fails_stays_restore_until_a_good_witness() {
    let mut f = restoring("sub/a.md");
    fail_root(&mut f);
    assert_eq!(f.save("sub/a.md"), Outcome::Published, "sub synced");
    assert_eq!(on_disk(&f, "sub/a.md").as_deref(), Some(OLD));
    assert_eq!(state(&f, "sub/a.md").0, PairPhase::Restore);
    assert_eq!(witness_failures(&f, "directorySync"), 1);
    retire_drafts(&mut f);
    assert!(drafted(&f, OLD));
    let mut f = reopen(f);
    assert_eq!(state(&f, "sub/a.md").0, PairPhase::Cancelled);
    settle_all(&mut f);
    assert!(!drafted(&f, OLD));
    assert_eq!(on_disk(&f, "sub/a.md").as_deref(), Some(OLD));
}

/// E4·U: newer text equal to the preimage, published while the root's
/// sync fails and the receiver is not certified, retains the pair
/// (Superseded, the draft keeps A's text). Use Disk on the receiver, once
/// the witness succeeds, cancels: the in-session exit for this state.
#[test]
fn a_disarm_whose_witness_fails_retains_until_use_disk_on_the_receiver() {
    let mut f = nested_pair();
    replace_outside(&f, "c.md", OLD);
    fail_root(&mut f);
    f.edit("sub/a.md", OLD);
    assert_eq!(f.save("sub/a.md"), Outcome::Published);
    assert_eq!(
        state(&f, "sub/a.md"),
        (PairPhase::Retained, PairChoice::Superseded)
    );
    assert_eq!(witness_failures(&f, "directorySync"), 1);
    retire_drafts(&mut f);
    assert!(drafted(&f, OLD));
    heal_root(&mut f);
    f.send("c.md", RequestKind::Discard);
    assert_eq!(
        state(&f, "sub/a.md"),
        (PairPhase::Cancelled, PairChoice::ReceiverDisk)
    );
}

/// E4·D (develop): the same publication with a good witness disarms.
#[test]
fn a_disarm_with_a_good_witness_disarms() {
    let mut f = nested_pair();
    replace_outside(&f, "c.md", OLD);
    f.edit("sub/a.md", OLD);
    assert_eq!(f.save("sub/a.md"), Outcome::Published);
    assert_eq!(state(&f, "sub/a.md").0, PairPhase::Disarmed);
}

/// E1 with an unreadable receiver (D×X / W×X): launch needs both
/// reads, so the pair stays Retained whatever the source's witness; the
/// launch after the receiver is readable again witnesses A and cancels.
#[test]
fn a_launch_with_an_unreadable_receiver_keeps_custody_until_it_reads() {
    let mut f = relaunched_after_a_crash_before_the_receiver_published();
    assert_eq!(f.save("c.md"), Outcome::Published);
    assert_eq!(f.save("b.md"), Outcome::Published);
    replace_outside(&f, "c.md", OLD);
    fail_root(&mut f);
    f.send("a.md", RequestKind::Discard);
    assert_eq!(
        state(&f, "a.md"),
        (PairPhase::Retained, PairChoice::SourceDisk)
    );
    unreadable(&f, "c.md");
    let root = f.graph.clone();
    let f = reopen_syncing(f, &[], &[(root, io::ErrorKind::Unsupported)]);
    assert_eq!(state(&f, "a.md").0, PairPhase::Retained, "W×X");
    let mut f = reopen(f);
    assert_eq!(state(&f, "a.md").0, PairPhase::Retained, "D×X");
    retire_drafts(&mut f);
    assert!(drafted(&f, OLD));
    fs::remove_dir(f.graph.join("c.md")).unwrap();
    replace_outside(&f, "c.md", OLD);
    let f = reopen(f);
    assert_eq!(state(&f, "a.md").0, PairPhase::Cancelled);
}

/// E7 split: a receiver read error at the removal's
/// pre-check fails the save and leaves the pair Armed (develop); it does
/// not hold the pair. Once C reads again with its certificate intact, Use
/// Disk on C takes C's disk text directly, even with A unreadable.
#[test]
fn a_receiver_read_error_at_the_pre_check_fails_the_save_and_keeps_the_pair_armed() {
    let mut f = relaunched_after_a_crash_before_the_receiver_published();
    assert_eq!(f.save("c.md"), Outcome::Published);
    assert_eq!(f.save("b.md"), Outcome::Published);
    (f.host.fs.read_faults).insert("c.md".into(), [io::ErrorKind::Other].into());
    assert_eq!(f.save("a.md"), Outcome::Failed);
    assert_eq!(state(&f, "a.md").0, PairPhase::Armed);
    assert_eq!(on_disk(&f, "a.md").as_deref(), Some(OLD));
    unreadable(&f, "a.md");
    f.send("c.md", RequestKind::Discard);
    assert!(!refused(&f, "c.md", Refusal::ReadFailed));
    assert_eq!(state(&f, "a.md"), (PairPhase::Armed, PairChoice::None));
}

/// E7's readable uncertified receiver still holds the pair (develop).
#[test]
fn a_readable_uncertified_receiver_at_the_pre_check_holds_the_pair() {
    let f = held_before_the_trash_step();
    assert_eq!(state(&f, "a.md").0, PairPhase::Held { removed: false });
}

/// In-place overwrite: another program overwrites the source in place between
/// the witness's read and its flush. The same handle's reread sees other
/// bytes: the witness fails (U), so Use Disk on the uncertified receiver
/// retains the pair, and the draft keeps A's text.
#[test]
fn a_source_overwritten_in_place_during_its_witness_keeps_custody() {
    let mut f = relaunched_after_a_crash_before_the_receiver_published();
    assert_eq!(f.save("c.md"), Outcome::Published);
    assert_eq!(f.save("b.md"), Outcome::Published);
    replace_outside(&f, "c.md", OLD);
    let path = f.graph.join("a.md");
    before_witness_step(WitnessStep::FileSync, move || {
        fs::write(&path, "- 0ld\n").unwrap();
    });
    f.send("c.md", RequestKind::Discard);
    assert_eq!(state(&f, "a.md").0, PairPhase::Retained);
    assert_eq!(witness_failures(&f, "changed"), 1);
    retire_drafts(&mut f);
    assert!(drafted(&f, OLD));
}

/// Reopen: the path is replaced by another file with the same bytes
/// after the chain's syncs (a reopen): the final check sees another file,
/// so the witness fails. The plain witness flushes once and reads twice.
/// A launch with no decided source is develop's directory pass: no
/// directory identity is taken and no evidence kept; with one decided
/// source, only its chain's directories are identified.
#[test]
fn a_launch_without_decided_sources_takes_no_witness_evidence() {
    use crate::page_host::production::{DIR_IDS, DIR_SYNCS};
    let root = tempfile::tempdir().unwrap();
    let (graph, app) = (root.path().join("graph"), root.path().join("app"));
    let trash = graph.join("logseq/.tine-trash/pages");
    fs::create_dir_all(&app).unwrap();
    let mut pages = BTreeSet::new();
    for dir in ["a", "b/c", "d"] {
        fs::create_dir_all(graph.join(dir)).unwrap();
        let page = format!("{dir}/p.md");
        fs::write(graph.join(&page), "- x\n").unwrap();
        pages.insert(page);
    }
    let mut io = ProductionIo::attach(&graph, &app, "ids", &trash);
    DIR_IDS.with(|ids| ids.set(0));
    DIR_SYNCS.with(|syncs| syncs.set(0));
    assert!(io.graph_launch(&pages, &[]).is_empty());
    assert_eq!(DIR_IDS.with(std::cell::Cell::get), 0);
    let plain = DIR_SYNCS.with(|syncs| syncs.replace(0));
    // Graph root, a, b, b/c, d.
    assert_eq!(plain, 5);
    let source = ("b/c/p.md".to_string(), Some(Arc::from(&b"- x\n"[..])));
    let witnessed = io.graph_launch(&pages, &[source]);
    assert!(!matches!(witnessed[0], SourceWitness::Failed(_)));
    // The source's chain (graph root, b, b/c), identified when synced and
    // again at the final check; the directory syncs are unchanged.
    assert_eq!(DIR_IDS.with(std::cell::Cell::get), 6);
    assert_eq!(DIR_SYNCS.with(std::cell::Cell::get), plain);
}

#[test]
fn the_witness_fails_when_the_path_names_another_file_afterwards() {
    let mut f = fixture();
    let (path, graph) = (f.graph.join("a.md"), f.graph.clone());
    WITNESS_READS.with(|reads| reads.set(0));
    crate::atomic_file::FILE_SYNCS.with(|syncs| syncs.set(0));
    // A successful witness is weak on Windows by design.
    let good = if cfg!(windows) {
        SourceWitness::Weak
    } else {
        SourceWitness::Durable
    };
    assert_eq!(f.host.fs.source_witnessed("a.md", &bytes(OLD)), good);
    assert_eq!(WITNESS_READS.with(std::cell::Cell::get), 2);
    assert_eq!(crate::atomic_file::FILE_SYNCS.with(std::cell::Cell::get), 1);
    before_witness_step(WitnessStep::Replaced, move || {
        let temp = graph.join(".a.outside");
        fs::write(&temp, OLD).unwrap();
        fs::rename(&temp, &path).unwrap();
    });
    assert_eq!(
        f.host.fs.source_witnessed("a.md", &bytes(OLD)),
        SourceWitness::Failed(WitnessStep::Replaced)
    );
    assert_eq!(
        f.host.fs.source_witnessed("a.md", &bytes("- other\n")),
        SourceWitness::Failed(WitnessStep::Changed)
    );
    crate::atomic_file::FAIL_FILE_SYNC.with(|fail| fail.set(true));
    assert_eq!(
        f.host.fs.source_witnessed("a.md", &bytes(OLD)),
        SourceWitness::Failed(WitnessStep::FileSync)
    );
    let root = f.graph.clone();
    f.host
        .fs
        .sync_faults
        .insert(root.clone(), io::ErrorKind::Unsupported);
    assert_eq!(
        f.host.fs.source_witnessed("a.md", &bytes(OLD)),
        SourceWitness::Weak
    );
    f.host.fs.sync_faults.insert(root, io::ErrorKind::Other);
    assert_eq!(
        f.host.fs.source_witnessed("a.md", &bytes(OLD)),
        SourceWitness::Failed(WitnessStep::DirectorySync)
    );
    for (step, count) in [
        ("replaced", 1),
        ("changed", 1),
        ("fileSync", 1),
        ("directorySync", 1),
    ] {
        assert_eq!(witness_failures(&f, step), count, "{step}");
    }
}

/// A carried review trace, weak-launch receiver certification: the formal
/// model certifies a receiver's matching bytes at a weak launch, the native
/// host has no witness of them after a relaunch. Native is the stricter
/// answer: the pair stays Retained and the draft keeps A's text (develop's
/// receiver rule; v3 does not touch it).
#[test]
fn a_weak_launch_does_not_certify_a_matching_receiver() {
    let mut f = relaunched_after_a_crash_before_the_receiver_published();
    assert_eq!(f.save("c.md"), Outcome::Published);
    assert_eq!(f.save("b.md"), Outcome::Published);
    replace_outside(&f, "c.md", OLD);
    fs::write(f.graph.join("a.md"), "- edited\n").unwrap();
    f.send("a.md", RequestKind::Discard);
    assert_eq!(
        state(&f, "a.md"),
        (PairPhase::Retained, PairChoice::SourceDisk)
    );
    let root = f.graph.clone();
    let mut f = reopen_syncing(f, &[], &[(root, io::ErrorKind::Unsupported)]);
    assert_eq!(state(&f, "a.md").0, PairPhase::Retained);
    retire_drafts(&mut f);
    assert!(drafted(&f, OLD));
}

/// A carried review trace, Restore after an external deletion with no
/// trash copy: native launch offers Restore for a decided pair whose source
/// is absent (its rule assumes the removal), while the formal model also
/// requires the preimage in the trash (Q3 eligibility) and retains. Either
/// way the draft keeps A's text, and restoration has no production caller.
#[test]
fn a_source_deleted_outside_without_a_trash_copy_restores_from_the_draft() {
    let mut f = relaunched_after_a_crash_before_the_receiver_published();
    assert_eq!(f.save("c.md"), Outcome::Published);
    assert_eq!(f.save("b.md"), Outcome::Published);
    replace_outside(&f, "c.md", "- other\n");
    fail_root(&mut f);
    f.send("a.md", RequestKind::Discard);
    assert_eq!(state(&f, "a.md").0, PairPhase::Retained);
    fs::remove_file(f.graph.join("a.md")).unwrap();
    assert!(trashed(&f).is_empty(), "no trash copy");
    let mut f = reopen(f);
    assert_eq!(state(&f, "a.md").0, PairPhase::Restore);
    retire_drafts(&mut f);
    assert!(drafted(&f, OLD));
}

/// Cost of a launch over 1,000 decided pairs (1,000 sources, 1,000
/// receivers) under 4 shared page directories: develop's launch (its
/// directory pass, then a byte read of each decided source), this witness
/// (the same reads, the file steps sharing the launch's one directory pass)
/// and a per-source witness (each source syncing its own chain). Measured,
/// not asserted beyond the counts the recipe fixes. Run with `--ignored
/// --nocapture`.
#[test]
#[ignore = "cost measurement"]
fn launch_witness_cost_for_1000_pairs_under_4_directories() {
    use crate::page_host::production::DIR_SYNCS;
    let root = tempfile::tempdir().unwrap();
    let (graph, app) = (root.path().join("graph"), root.path().join("app"));
    let trash = graph.join("logseq/.tine-trash/pages");
    fs::create_dir_all(&app).unwrap();
    let mut pages = BTreeSet::new();
    let mut sources = Vec::new();
    for dir in 0..4 {
        fs::create_dir_all(graph.join(format!("d{dir}"))).unwrap();
        for i in 0..250 {
            let (source, receiver) = (format!("d{dir}/s{i}.md"), format!("d{dir}/r{i}.md"));
            let text = format!("- page {dir}/{i}\n");
            fs::write(graph.join(&source), &text).unwrap();
            fs::write(graph.join(&receiver), &text).unwrap();
            pages.extend([source.clone(), receiver]);
            sources.push((source, Some(Arc::from(text.into_bytes()))));
        }
    }
    let counts = || {
        (
            DIR_SYNCS.with(|syncs| syncs.replace(0)),
            crate::atomic_file::FILE_SYNCS.with(|syncs| syncs.replace(0)),
            WITNESS_READS.with(|reads| reads.replace(0)),
        )
    };
    let mut io = ProductionIo::attach(&graph, &app, "cost", &trash);
    let mut measure = |name: &str, run: &mut dyn FnMut(&mut ProductionIo) -> usize| {
        counts();
        let start = std::time::Instant::now();
        let held = run(&mut io);
        let elapsed = start.elapsed();
        let (dirs, files, witness_reads) = counts();
        let reads = sources.len() as u64 + witness_reads;
        eprintln!(
            "COST {name}: directory syncs {dirs}, file fsyncs {files}, file reads {reads}, held {held}, {elapsed:?}"
        );
        (dirs, files, reads)
    };
    let read_sources = |io: &mut ProductionIo| {
        (sources.iter())
            .filter(|(page, bytes)| io.read_page(page).ok().as_ref() == Some(bytes))
            .count()
    };
    let develop = measure("develop", &mut |io| {
        io.graph_launch(&pages, &[]);
        read_sources(io)
    });
    let candidate = measure("candidate", &mut |io| {
        read_sources(io);
        let witnessed = io.graph_launch(&pages, &sources);
        witnessed
            .iter()
            .filter(|w| !matches!(w, SourceWitness::Failed(_)))
            .count()
    });
    let per_source = measure("per-source", &mut |io| {
        io.graph_launch(&pages, &[]);
        read_sources(io);
        (sources.iter())
            .filter(|(page, bytes)| {
                !matches!(io.source_witnessed(page, bytes), SourceWitness::Failed(_))
            })
            .count()
    });
    // Graph root and the 4 shared directories, once each (no trash yet).
    assert_eq!(develop, (5, 0, 1000));
    assert_eq!(candidate, (5, 1000, 3000));
    assert_eq!(per_source, (5 + 2 * 1000, 1000, 3000));
}
