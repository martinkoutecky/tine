use super::io::Phase;
use super::model_fs::{Fault, ModelFs};
use super::*;

#[path = "mutation_tests.rs"]
mod mutation_tests;
#[path = "progress_tests.rs"]
mod progress_tests;

pub(super) fn text(bytes: &str) -> Text {
    Some(Arc::from(bytes.as_bytes()))
}

#[test]
fn review_f6_late_switch_ready_after_stop_cannot_apply() {
    let mut h = host();
    h.stop();
    assert_eq!(h.switch_ready(0), Disposition::Disabled);
    assert!(h.switch_confirmation.is_none());
    assert!(h.admission_open);
    assert!(!h.can_switch());
    h.switch_abort();
    assert_eq!(h.switch_ready(0), Disposition::Disabled);
    assert!(!h.can_switch());
    assert_eq!(h.launch(), Disposition::Applied);
    assert_eq!(h.switch_ready(0), Disposition::Applied);
    assert!(h.can_switch());
    assert_eq!(h.switch_finish(), Disposition::Applied);
    assert_eq!(h.switch_ready(0), Disposition::Disabled);
    assert!(!h.can_switch());
}

#[test]
fn review_f8_production_rename_rebinds_title_with_durable_custody_and_recovery() {
    for power in [false, true] {
        let mut h = host();
        h.fs.external("a.md", text("title:: A\n- body\n"), true);
        h.fs.external("b.md", text("title:: Referrer\n- [[A]]\n"), true);
        assert_eq!(
            h.rename(
                "a.md",
                "c.md",
                &BTreeSet::from(["b.md".into()]),
                "A",
                "C",
                tine_core::config::FileNameFormat::TripleLowbar
            ),
            Disposition::Pending
        );
        assert!(h.pages.values().all(Page::clean));
        assert_eq!(h.start_save("c.md"), Disposition::Waiting);
        for _ in 0..3 {
            h.advance_draft();
        }
        assert!(
            h.pages.values().all(Page::clean),
            "durable but unapplied vehicle retains old buffers"
        );
        h.advance_draft();
        assert_eq!(h.pages["c.md"].buf, text("title:: C\n- body\n"));
        assert_eq!(h.pages["a.md"].buf, None);
        assert_eq!(h.pages["b.md"].buf, text("title:: Referrer\n- [[C]]\n"));
        let records = h.logical_drafts();
        assert_eq!(records["c.md"].bytes, h.pages["c.md"].buf);
        assert_eq!(records["b.md"].bytes, h.pages["b.md"].buf);
        assert_eq!(h.start_save("c.md"), Disposition::Waiting);
        restart(&mut h, power, false);
        drain(&mut h);
        assert_eq!(h.pages["c.md"].buf, text("title:: C\n- body\n"));
        saved(&mut h, "c.md");
        assert_eq!(h.fs.files["graph/c.md"].as_ref(), b"title:: C\n- body\n");
    }
}

pub(super) fn host() -> Host<ModelFs> {
    let mut fs = ModelFs::default();
    fs.external("a.md", text("A"), true);
    fs.external("b.md", text("B"), true);
    let locks = ["a.md", "b.md", "c.md"]
        .into_iter()
        .map(|key| (key.into(), Arc::new(Mutex::new(()))))
        .collect();
    Host::new(fs, locks)
}

pub(super) fn send(host: &mut Host<ModelFs>, key: &str, kind: RequestKind) -> Disposition {
    let request = Request {
        id: host.last_admitted + 1,
        generation: host.generation,
        page: key.into(),
        kind,
    };
    assert_eq!(host.admit(request), Disposition::Applied);
    assert_eq!(host.dequeue(), Disposition::Pending);
    host.apply_request()
}

pub(super) fn open(host: &mut Host<ModelFs>, key: &str) {
    assert_eq!(send(host, key, RequestKind::Open), Disposition::Applied);
    host.receive(key);
}

pub(super) fn edit(host: &mut Host<ModelFs>, key: &str, bytes: &str) {
    let version = host.pages[key].version;
    assert_eq!(
        send(
            host,
            key,
            RequestKind::Submit {
                bytes: text(bytes),
                version,
                resolve: None
            }
        ),
        Disposition::Applied
    );
    host.receive(key);
}

pub(super) fn drain(host: &mut Host<ModelFs>) {
    for _ in 0..200 {
        if host.worker.is_none() {
            return;
        }
        host.advance_draft();
    }
    panic!("draft worker failed to reach terminal state on a recovered disk");
}

pub(super) fn draft(host: &mut Host<ModelFs>, key: &str) {
    assert_eq!(host.begin_draft(key), Disposition::Pending);
    drain(host);
}

pub(super) fn risk(host: &mut Host<ModelFs>, key: &str) {
    assert_eq!(host.start_save(key), Disposition::Pending);
    host.fs.inject(Phase::PageTemp, [Fault::Before]);
    host.advance_save(0);
    assert!(host.pages[key].risk);
}

pub(super) fn saved(host: &mut Host<ModelFs>, key: &str) {
    assert_eq!(host.start_save(key), Disposition::Pending);
    for _ in 0..10 {
        if host.job.is_none() {
            break;
        }
        let epoch = host.fs.epochs.get(key).copied().unwrap_or(0);
        host.advance_save(epoch);
    }
    assert!(host.pages[key].clean());
}

pub(super) fn restart(host: &mut Host<ModelFs>, power: bool, keep_drafts: bool) {
    host.stop();
    if power {
        host.fs.power(&BTreeSet::new(), keep_drafts);
    } else {
        host.fs.crash();
    }
    assert!(matches!(
        host.launch(),
        Disposition::Applied | Disposition::Pending
    ));
    drain(host);
}

#[test]
fn custody_survives_dequeue_and_window_generation_change() {
    let mut h = host();
    open(&mut h, "a.md");
    let version = h.pages["a.md"].version;
    let request = Request {
        id: 2,
        generation: h.generation,
        page: "a.md".into(),
        kind: RequestKind::Submit {
            bytes: text("unsaved"),
            version,
            resolve: None,
        },
    };
    h.admit(request.clone());
    assert_eq!(h.abstract_queue(), vec![request.clone()]);
    h.dequeue();
    assert_eq!(h.abstract_queue(), vec![request]);
    h.window_crash();
    assert_eq!(h.apply_request(), Disposition::Applied);
    assert_eq!(h.pages["a.md"].buf, text("unsaved"));
    assert!(h.outbox.is_empty());
}

#[test]
fn outbox_coalesces_newest_push_with_unread_answer() {
    let mut h = host();
    assert_eq!(
        send(&mut h, "a.md", RequestKind::Open),
        Disposition::Applied
    );
    let answer = h.outbox["a.md"].answer.clone();
    h.fs.external("a.md", text("external"), true);
    h.observe("a.md");
    let mail = h.receive("a.md").unwrap();
    assert_eq!(mail.answer, answer);
    assert_eq!(mail.page.unwrap().buf, text("external"));
}

#[test]
fn stale_submit_keeps_unknown_base_and_equal_byte_read_keeps_risk() {
    let mut h = host();
    open(&mut h, "a.md");
    let old = h.pages["a.md"].version;
    h.fs.external("a.md", text("external"), true);
    h.observe("a.md");
    send(
        &mut h,
        "a.md",
        RequestKind::Submit {
            bytes: text("mine"),
            version: old,
            resolve: None,
        },
    );
    assert_eq!(h.pages["a.md"].base, Base::Unknown);
    assert!(h.pages["a.md"].conflict);
    h.fs.external("a.md", text("mine"), true);
    h.observe("a.md");
    assert_eq!(h.pages["a.md"].base, Base::Known(text("mine")));
    assert!(h.pages["a.md"].risk && h.pages["a.md"].typed);
    assert!(!h.pages["a.md"].conflict);
}

#[test]
fn equal_bytes_refresh_selects_wseq_at_same_version_in_both_orders() {
    for reverse in [false, true] {
        let mut h = host();
        open(&mut h, "a.md");
        edit(&mut h, "a.md", "mine");
        risk(&mut h, "a.md");
        draft(&mut h, "a.md");
        let old = h.logical_drafts()["a.md"].clone();
        h.fs.external("a.md", text("mine"), true);
        h.observe("a.md");
        h.begin_draft("a.md");
        for _ in 0..3 {
            h.advance_draft();
        }
        assert_eq!(h.logical_drafts()["a.md"], old, "durable but not applied");
        h.advance_draft();
        let new = h.logical_drafts()["a.md"].clone();
        assert_eq!(old.version, new.version);
        assert!(new.wseq > old.wseq);
        assert_ne!(new.base, old.base);
        let mut files = h.fs.draft_files(false);
        if reverse {
            files.reverse();
        }
        assert_eq!(drafts::scan(files).logical["a.md"], new);
        restart(&mut h, false, false);
        assert_eq!(h.pages["a.md"].base, Base::Known(text("mine")));
    }
}

#[test]
fn failed_move_never_removes_receivers_previous_draft_at_any_cut() {
    for cut in 0..12 {
        for power in [false, true] {
            let mut h = host();
            open(&mut h, "a.md");
            open(&mut h, "b.md");
            edit(&mut h, "b.md", "previous promised");
            risk(&mut h, "b.md");
            draft(&mut h, "b.md");
            let original = h.logical_drafts()["b.md"].clone();
            let sv = h.pages["a.md"].version;
            let dv = h.pages["b.md"].version;
            h.fs.inject(
                Phase::DraftSync,
                [Fault::Before, Fault::Before, Fault::Before],
            );
            assert_eq!(
                send(
                    &mut h,
                    "a.md",
                    RequestKind::Move {
                        receiver: "b.md".into(),
                        source_text: text("source remainder"),
                        receiver_text: text("moved"),
                        source_version: sv,
                        receiver_version: dv,
                    }
                ),
                Disposition::Pending
            );
            for _ in 0..cut {
                h.advance_draft();
            }
            if h.worker.is_none() {
                assert_eq!(h.logical_drafts()["b.md"], original);
                assert!(!h.outbox["b.md"].answer.as_ref().unwrap().took);
            }
            restart(&mut h, power, false);
            let recovered = h.pages["b.md"].buf.clone();
            if !power && (2..=5).contains(&cut) {
                assert_eq!(recovered, text("moved"));
            } else {
                assert_eq!(
                    recovered,
                    text("previous promised"),
                    "cut {cut}, power {power}"
                );
            }
        }
    }
}

#[test]
fn pending_operation_reserves_allocator_and_custody_until_application() {
    let mut h = host();
    open(&mut h, "a.md");
    open(&mut h, "b.md");
    open(&mut h, "c.md");
    let version = h.version;
    let a = h.pages["a.md"].version;
    let b = h.pages["b.md"].version;
    send(
        &mut h,
        "a.md",
        RequestKind::Move {
            receiver: "b.md".into(),
            source_text: text("rest"),
            receiver_text: text("move"),
            source_version: a,
            receiver_version: b,
        },
    );
    let request = h.abstract_queue()[0].clone();
    assert!(h.allocator_busy());
    assert_eq!(h.version, version);
    assert_eq!(h.observe("c.md"), Disposition::Waiting);
    for _ in 0..3 {
        h.advance_draft();
    }
    assert_eq!(h.abstract_queue(), vec![request]);
    assert_eq!(h.pages["b.md"].buf, text("B"));
    assert!(h.outbox.is_empty());
    h.advance_draft();
    assert_eq!(h.pages["b.md"].version, version + 1);
    assert_eq!(h.pages["a.md"].version, version + 2);
    assert!(h.abstract_queue().is_empty());
    drain(&mut h);
    assert!(!h.allocator_busy());
}

#[test]
fn move_changes_receivers_draft_only_when_source_was_already_at_risk() {
    let mut h = host();
    open(&mut h, "a.md");
    open(&mut h, "b.md");
    edit(&mut h, "a.md", "old source draft");
    risk(&mut h, "a.md");
    draft(&mut h, "a.md");
    let old = h.logical_drafts()["a.md"].clone();
    let a = h.pages["a.md"].version;
    let b = h.pages["b.md"].version;
    send(
        &mut h,
        "a.md",
        RequestKind::Move {
            receiver: "b.md".into(),
            source_text: text("new source remainder"),
            receiver_text: text("received"),
            source_version: a,
            receiver_version: b,
        },
    );
    drain(&mut h);
    assert_eq!(h.logical_drafts()["a.md"], old);
    assert_eq!(h.pages["a.md"].buf, text("new source remainder"));
    draft(&mut h, "a.md");
    assert_eq!(
        h.logical_drafts()["a.md"].bytes,
        text("new source remainder")
    );
}

#[test]
fn post_unlink_sync_failure_retries_sync_only_for_cleanup_retirement_and_removal() {
    for cleanup in [false, true] {
        let mut fs = ModelFs::default();
        let record = Record {
            page: "a.md".into(),
            wseq: 1,
            version: 1,
            base: Base::Unknown,
            bytes: text("mine"),
        };
        let mut task = Vehicle::write(drafts::page_name("a.md"), &[record]);
        for _ in 0..3 {
            task.advance(&mut fs);
        }
        task.advance(&mut fs);
        assert_eq!(task.stage, Stage::Present);
        task.stage = if cleanup {
            Stage::CleanupUnlink
        } else {
            Stage::Unlink
        };
        fs.inject(Phase::DraftSync, [Fault::Before]);
        task.advance(&mut fs);
        task.advance(&mut fs);
        let unlink_calls = fs
            .calls
            .iter()
            .filter(|p| **p == Phase::DraftUnlink)
            .count();
        assert_eq!(fs.draft_files(false).len(), 0);
        assert_eq!(fs.draft_files(true).len(), 1);
        task.advance(&mut fs);
        assert_eq!(task.stage, Stage::Absent);
        assert!(fs.draft_files(true).is_empty());
        assert_eq!(
            fs.calls
                .iter()
                .filter(|p| **p == Phase::DraftUnlink)
                .count(),
            unlink_calls
        );
    }
}

#[test]
fn oldest_first_removal_never_uncovers_older_record_and_abs_retains_newest() {
    for cut in 0..10 {
        let mut h = host();
        open(&mut h, "a.md");
        edit(&mut h, "a.md", "mine");
        risk(&mut h, "a.md");
        draft(&mut h, "a.md");
        let latest = h.logical_drafts()["a.md"].clone();
        let mut older = latest.clone();
        older.wseq -= 1;
        // Use a positive, globally distinct sequence for the older vehicle.
        let mut newest = latest.clone();
        newest.wseq = 3;
        older.wseq = 1;
        h.fs.files.clear();
        h.fs.stable.clear();
        for r in [older, newest.clone()] {
            let name = format!("draft/{}", drafts::page_name("a.md"));
            let bytes: Arc<[u8]> = Arc::from(drafts::encode(&[r]));
            h.fs.files.insert(name.clone(), bytes.clone());
            h.fs.stable.insert(name, bytes);
        }
        // User discard ends the current draft obligation when disk differs.
        h.fs.external("a.md", text("different"), true);
        send(&mut h, "a.md", RequestKind::Discard { version: 0 });
        assert!(!h.pages["a.md"].risk);
        h.begin_draft("a.md");
        for _ in 0..cut {
            h.advance_draft();
        }
        if h.worker.is_some() {
            assert_eq!(h.logical_drafts()["a.md"], newest);
        }
        let physical = drafts::scan(h.fs.draft_files(true)).logical;
        assert!(physical.get("a.md").is_none_or(|r| r.wseq == 3));
        restart(&mut h, true, false);
        assert!(h.pages.get("a.md").is_none_or(|p| p.buf == text("mine")));
    }
}

#[test]
fn save_guard_mismatch_and_uncertain_never_report_clean() {
    let mut h = host();
    open(&mut h, "a.md");
    edit(&mut h, "a.md", "mine");
    h.start_save("a.md");
    h.advance_save(0);
    h.fs.external("a.md", text("external"), true);
    h.advance_save(1);
    assert!(h.job.is_none());
    assert!(h.pages["a.md"].conflict && h.pages["a.md"].risk);
    let version = h.pages["a.md"].version;
    send(
        &mut h,
        "a.md",
        RequestKind::Submit {
            bytes: text("mine"),
            version,
            resolve: Some(text("external")),
        },
    );
    h.start_save("a.md");
    h.advance_save(0);
    h.advance_save(0);
    h.advance_save(1);
    h.fs.inject(Phase::PageSync, [Fault::Before]);
    h.advance_save(1);
    assert!(h.pages["a.md"].risk && h.pages["a.md"].typed);
    assert_eq!(h.pages["a.md"].base, Base::Known(text("external")));
    assert_eq!(h.fs.files["graph/a.md"].as_ref(), b"mine");
    assert_eq!(h.fs.stable["graph/a.md"].as_ref(), b"external");
}

#[test]
fn deletion_durably_copies_trash_before_source_removal_at_power_cut() {
    let mut h = host();
    assert_eq!(h.delete("a.md"), Disposition::Pending);
    drain(&mut h);
    h.start_save("a.md");
    h.advance_save(0); // guard
    h.advance_save(0); // marker
    h.advance_save(0); // move
    h.advance_save(0); // custody (a)+(b)
    assert_eq!(h.job.as_ref().unwrap().phase, SavePhase::DirectorySync);
    assert!(h.fs.stable.contains_key("graph/a.md"));
    assert!(h.fs.stable.keys().any(|k| k.starts_with("trash/a.md/")));
    restart(&mut h, true, false);
    assert_eq!(h.fs.files["graph/a.md"].as_ref(), b"A");
    assert!(h.fs.files.keys().any(|k| k.starts_with("trash/a.md/")));
    assert_eq!(h.pages["a.md"].buf, None);
}

#[test]
fn completed_trash_move_error_keeps_observed_removal_and_reports_uncertain() {
    let mut h = host();
    h.delete("a.md");
    drain(&mut h);
    h.start_save("a.md");
    h.advance_save(0); // guard
    h.advance_save(0); // marker
    h.fs.inject(Phase::TrashMove, [Fault::After]);
    h.advance_save(0);
    assert!(h.job.is_none());
    assert!(h.pages["a.md"].risk);
    assert!(!h.fs.files.contains_key("graph/a.md"));
    assert!(h.events.contains(&Event::Removed {
        page: "a.md".into(),
        bytes: text("A")
    }));
    assert!(h.events.contains(&Event::SaveOutcome {
        page: "a.md".into(),
        outcome: Outcome::Uncertain
    }));
}

#[test]
fn repeated_launch_explosion_retains_one_identical_copy_per_page() {
    let mut h = host();
    h.delete("a.md");
    for _ in 0..4 {
        h.advance_draft();
    }
    // Cut after the explosion copy is durable, before op retirement.
    for _ in 0..3 {
        h.advance_draft();
    }
    assert_eq!(h.fs.draft_files(true).len(), 2);
    restart(&mut h, false, false);
    assert_eq!(h.fs.draft_files(true).len(), 1);
    restart(&mut h, false, false);
    assert_eq!(h.fs.draft_files(true).len(), 1);
}

#[test]
fn operation_explodes_before_save_and_retries_failed_copy() {
    let mut h = host();
    h.fs.external("b.md", text("[[A]] unrelated"), true);
    open(&mut h, "b.md");
    assert_eq!(
        h.rename(
            "a.md",
            "c.md",
            &BTreeSet::new(),
            "A",
            "C",
            tine_core::config::FileNameFormat::TripleLowbar
        ),
        Disposition::Pending
    );
    for _ in 0..5 {
        h.advance_draft();
    }
    assert_eq!(h.pages["b.md"].buf, text("[[C]] unrelated"));
    assert_eq!(h.start_save("c.md"), Disposition::Waiting);
    h.fs.inject(Phase::DraftTemp, [Fault::Before]);
    drain(&mut h);
    let scan = drafts::scan(h.fs.draft_files(true));
    assert_eq!(scan.logical.len(), 3);
    assert!(scan.files.keys().all(|name| name.starts_with("p-")));
    saved(&mut h, "c.md");
}

#[test]
fn unclean_referrer_refuses_rename_without_io_or_buffer_side_effects() {
    let mut h = host();
    open(&mut h, "b.md");
    edit(&mut h, "b.md", "[[A]] mine");
    let before = h.pages.clone();
    let version = h.version;
    assert_eq!(
        h.rename(
            "a.md",
            "c.md",
            &BTreeSet::from(["b.md".into()]),
            "A",
            "C",
            tine_core::config::FileNameFormat::TripleLowbar
        ),
        Disposition::Refused
    );
    assert_eq!(h.pages, before);
    assert_eq!(h.version, version);
    assert!(h.fs.draft_files(false).is_empty());
}

#[test]
fn switch_waits_for_final_answer_and_durable_draft_removal() {
    let mut h = host();
    open(&mut h, "a.md");
    let version = h.pages["a.md"].version;
    send(
        &mut h,
        "a.md",
        RequestKind::Submit {
            bytes: text("mine"),
            version,
            resolve: None,
        },
    );
    assert_eq!(h.switch_ready(h.last_applied), Disposition::Waiting);
    h.receive("a.md");
    assert_eq!(h.switch_ready(h.last_applied), Disposition::Applied);
    h.switch_request();
    draft(&mut h, "a.md");
    assert!(h.can_switch());
    saved(&mut h, "a.md");
    assert!(!h.can_switch());
    draft(&mut h, "a.md");
    assert!(h.can_switch());
    h.switch_abort();
    assert!(h.admission_open);
}

#[test]
fn launch_seeds_all_readable_versions_and_canonical_unknown_base() {
    let mut h = host();
    let records = [
        Record {
            page: "a.md".into(),
            wseq: 10,
            version: 80,
            base: Base::Unknown,
            bytes: text("mine"),
        },
        Record {
            page: "c.md".into(),
            wseq: 11,
            version: 50,
            base: Base::Known(None),
            bytes: None,
        },
    ];
    let name = drafts::op_name();
    h.fs.draft_temp(&name, &drafts::encode(&records)).unwrap();
    h.fs.draft_rename(&name).unwrap();
    restart(&mut h, false, false);
    assert_eq!(h.pages["a.md"].version, 81);
    assert_eq!(h.pages["c.md"].version, 82);
    assert_eq!(h.wseq, 11);
    let a = &h.pages["a.md"];
    assert!(a.typed && a.risk && !a.conflict);
    assert_eq!(a.obs, None);
    assert_eq!(a.base, Base::Unknown);
    assert!(h
        .fs
        .draft_files(true)
        .iter()
        .all(|(name, _)| name.starts_with("p-")));
}

/// STEP3 §1: launch plans its path locks before any side effect, so the
/// driver's revalidating poll is the first to quarantine or report.
#[test]
fn launch_takes_its_locks_before_quarantining_an_unreadable_vehicle() {
    let mut h = host();
    let record = Record {
        page: "a.md".into(),
        wseq: 1,
        version: 1,
        base: Base::Unknown,
        bytes: text("one"),
    };
    let name = drafts::page_name("a.md");
    h.fs.draft_temp(&name, &drafts::encode(&[record])).unwrap();
    h.fs.draft_rename(&name).unwrap();
    h.fs.files.insert(
        "draft/p-corrupt.draft".into(),
        Arc::from(b"torn".as_slice()),
    );
    h.stop();
    h.fs.crash();
    h.held = Some(BTreeSet::new());
    assert_eq!(h.launch(), Disposition::Waiting);
    assert_eq!(h.lock_request, Some(BTreeSet::from(["a.md".into()])));
    assert!(h.fs.files.contains_key("draft/p-corrupt.draft"));
    assert!(h.events.is_empty());
    h.held = h.lock_request.take();
    assert!(matches!(
        h.launch(),
        Disposition::Applied | Disposition::Pending
    ));
    assert!(!h.fs.files.contains_key("draft/p-corrupt.draft"));
    assert_eq!(h.pages["a.md"].buf, text("one"));
}

/// M2 (plan v3 §4) on the model filesystem: process-crash survivors that
/// launch cannot sync stay buffered at risk and unretired, every draft
/// effect fails without touching a file, and the census claims no
/// durability. A Retry's sync makes them durable and retires the older.
#[test]
fn m2_an_unsynced_launch_retires_nothing_until_a_retry_syncs_the_census() {
    let mut h = host();
    let mut names = vec![];
    for (wseq, bytes) in [(1, "older"), (2, "newest")] {
        let record = Record {
            page: "a.md".into(),
            wseq,
            version: wseq,
            base: Base::Unknown,
            bytes: text(bytes),
        };
        let name = drafts::page_name("a.md");
        h.fs.draft_temp(&name, &drafts::encode(&[record])).unwrap();
        h.fs.draft_rename(&name).unwrap();
        names.push(name);
    }
    h.stop();
    h.fs.crash();
    h.fs.inject(Phase::DraftSync, [Fault::Before]);
    h.held = Some(BTreeSet::from(["a.md".into()]));
    assert_eq!(h.launch(), Disposition::Applied);
    assert!(h.alive && h.worker.is_none());
    assert!(h.fs.draft_status().unavailable.is_some());
    assert!(h.pages["a.md"].risk && h.pages["a.md"].typed);
    assert_eq!(h.pages["a.md"].buf, text("newest"));
    assert!(h.fs.draft_files(true).is_empty());
    let files = h.fs.files.clone();
    assert!(h.fs.draft_unlink(&names[0]).is_err());
    assert!(h.fs.draft_temp("p-new.draft", b"x").is_err());
    assert_eq!(h.fs.files, files);
    assert_eq!(h.drafts_retry(), Ok(Disposition::Pending));
    while h.worker.is_some() {
        h.advance_draft();
    }
    assert_eq!(h.fs.draft_status(), DraftStatus::default());
    let durable: Vec<_> = h.fs.draft_files(true).into_iter().map(|(n, _)| n).collect();
    assert_eq!(durable, [names[1].clone()]);
    assert_eq!(h.pages["a.md"].buf, text("newest"));
}

#[test]
fn unreadable_files_are_preserved_and_equal_wseq_disagreement_is_quarantined() {
    let mut h = host();
    let one = Record {
        page: "a.md".into(),
        wseq: 1,
        version: 1,
        base: Base::Unknown,
        bytes: text("one"),
    };
    let mut two = one.clone();
    two.bytes = text("two");
    for records in [vec![one], vec![two]] {
        let name = drafts::page_name("a.md");
        h.fs.draft_temp(&name, &drafts::encode(&records)).unwrap();
        h.fs.draft_rename(&name).unwrap();
    }
    h.fs.files.insert(
        "draft/p-corrupt.draft".into(),
        Arc::from(b"torn".as_slice()),
    );
    restart(&mut h, false, false);
    assert!(h.pages.is_empty());
    assert_eq!(
        h.fs.files
            .keys()
            .filter(|k| k.starts_with("unreadable/"))
            .count(),
        3
    );
    assert_eq!(
        h.events
            .iter()
            .filter(|e| matches!(e, Event::Unreadable(_)))
            .count(),
        3
    );
}

#[test]
fn weak_graph_does_not_weaken_app_data_and_power_reverts_only_unsynced_names() {
    let mut h = host();
    h.fs.weak_graph = true;
    open(&mut h, "a.md");
    edit(&mut h, "a.md", "mine");
    risk(&mut h, "a.md");
    draft(&mut h, "a.md");
    saved(&mut h, "a.md");
    assert_eq!(h.fs.stable["graph/a.md"].as_ref(), b"A");
    restart(&mut h, true, false);
    assert_eq!(h.pages["a.md"].buf, text("mine"));
    assert_eq!(h.fs.files["graph/a.md"].as_ref(), b"A");
}

#[test]
fn retained_reservation_blocks_save_and_reconciles_undo_or_publication() {
    let mut h = host();
    open(&mut h, "a.md");
    let keys = BTreeSet::from(["a.md".into()]);
    assert_eq!(h.reserve(&keys), Disposition::Applied);
    assert_eq!(h.start_save("a.md"), Disposition::Waiting);
    h.fs.external("a.md", text("transaction"), true);
    assert_eq!(h.release(&keys), Disposition::Applied);
    assert_eq!(h.pages["a.md"].buf, text("transaction"));
}

/// The production files that name the page host surface: the census
/// writers (STEP3 §7), the binding's host slot, restore and retirement, the
/// page commands and the `load_graph` reply (step 3b P1).
const CENSUS_CALL_SITES: &[&str] = &[
    "crates/tine-graph-features/src/retained.rs",
    "crates/tine-graph-features/src/pages.rs",
    "crates/tine-graph-features/src/conflicts.rs",
    "crates/tine-graph-features/src/journals.rs",
    "crates/tine-graph-features/src/pdf.rs",
    "crates/tine-graph-features/src/live_conflict.rs",
    "crates/tine-graph-features/src/guide.rs",
    "src-tauri/src/state.rs",
    "src-tauri/src/backup/restore.rs",
    "src-tauri/src/host_retirement.rs",
    "src-tauri/src/page_commands.rs",
    "src-tauri/src/graph.rs",
];

#[test]
fn host_and_oracle_stay_private_unwired_and_runtime_has_no_filesystem_escape() {
    let root = option_env!("TINE_HOST_REPO_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
    /// Whether `parent` declares the sibling `file` as a module, under
    /// `#[cfg(test)]` when `gated`.
    fn declares(parent: &str, file: &str, gated: bool) -> bool {
        let stem = file.trim_end_matches(".rs");
        let lines: Vec<_> = parent.lines().collect();
        lines.iter().enumerate().any(|(i, line)| {
            let named = *line == format!("mod {stem};") || *line == format!("#[path = \"{file}\"]");
            named && (!gated || (i > 0 && lines[i - 1] == "#[cfg(test)]"))
        })
    }
    fn visit(root: &std::path::Path, dir: &std::path::Path) {
        let paths: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        let siblings: Vec<String> = paths
            .iter()
            .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
            .map(|path| std::fs::read_to_string(path).unwrap())
            .collect();
        for path in paths {
            if path.is_dir() {
                visit(root, &path);
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "rs") {
                continue;
            }
            // `/`-joined on every platform: Windows paths display with `\`.
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            // An integration test crate cannot name a private module; there
            // "page_host" is only a path string (the I-21 owner census).
            if relative.contains("/page_state/")
                || relative.contains("/page_host/")
                || relative.contains("/tests/")
            {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            let name = path.file_name().unwrap().to_string_lossy();
            // Test code: a file a sibling declares under `#[cfg(test)]`, and a
            // top-level `#[cfg(test)] mod … {` up to its closing `}`.
            let mut test_code = siblings.iter().any(|parent| declares(parent, &name, true));
            let test_file = test_code;
            let lines: Vec<&str> = source.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                if *line == "#[cfg(test)]"
                    && lines
                        .get(i + 1)
                        .is_some_and(|next| next.starts_with("mod ") && next.ends_with('{'))
                {
                    test_code = true;
                } else if *line == "}" && !test_file {
                    test_code = false;
                }
                // The retained-writer surface (approved by Martin 2026-10-10) is named only
                // where a census writer reserves from a host or the binding
                // keeps one (STEP3 §7), and no production path starts a host
                // while the switch is off (lane 3b owns the switch).
                assert!(
                    test_code
                        || (!line.contains("start_for_tests") && !line.contains("PageHost::start")),
                    "a production path starts a page host: {relative}: {line}"
                );
                let exported = [
                    "DiskToken",
                    "DraftStatus",
                    "Input",
                    // `Opened` too, unlisted: a common word (capture_target,
                    // flight_store), reached only through `PageHost::open`.
                    "PageHost",
                    "PageMail",
                    "PageOperation",
                    "PageRefusal",
                    "Reloaded",
                    "RenameRefusal",
                    "Reservation",
                    "StopMode",
                    "StopState",
                    "Stopped",
                ]
                .into_iter()
                .find(|name| {
                    line.match_indices(name).any(|(at, _)| {
                        let word =
                            |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
                        !word(line[..at].chars().next_back())
                            && !word(line[at + name.len()..].chars().next())
                    })
                });
                if let Some(name) = exported.filter(|_| !test_code) {
                    assert!(
                        CENSUS_CALL_SITES.contains(&relative.as_str())
                            || relative.starts_with("crates/tine-store/"),
                        "page host name {name} outside the census-writer call sites \
                         (STEP3 §7; exemplar crates/tine-graph-features/src/conflicts.rs \
                         fold_pair): {relative}: {line}"
                    );
                }
                if relative == "crates/tine-store/src/lib.rs"
                    && [
                        "mod page_state;",
                        "mod page_host;",
                        // The Page host export (concept approved by Martin 2026-10-10).
                        "pub use page_host::{",
                    ]
                    .contains(&line.trim())
                {
                    continue;
                }
                assert!(
                    !line.contains("page_state") && !line.contains("page_host"),
                    "production consumer: {relative}: {line}"
                );
            }
        }
    }
    visit(&root, &root.join("crates"));
    visit(&root, &root.join("src-tauri/src"));
    let runtime = [
        ("mod.rs", include_str!("mod.rs")),
        ("binding.rs", include_str!("binding.rs")),
        ("binding_retained.rs", include_str!("binding_retained.rs")),
        (
            "binding_publication.rs",
            include_str!("binding_publication.rs"),
        ),
        ("binding_launch.rs", include_str!("binding_launch.rs")),
        ("draft_worker.rs", include_str!("draft_worker.rs")),
        ("drafts.rs", include_str!("drafts.rs")),
        ("driver.rs", include_str!("driver.rs")),
        ("io.rs", include_str!("io.rs")),
        ("operations.rs", include_str!("operations.rs")),
        ("progress.rs", include_str!("progress.rs")),
        ("save.rs", include_str!("save.rs")),
    ];
    // Every runtime file is scanned: a file split out of a scanned one must
    // join this list. Any other file here is the adapter, or a module
    // declared under `#[cfg(test)]` or inside such a module.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/page_host");
    let mut test_only: BTreeSet<String> = BTreeSet::new();
    let files: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    loop {
        let before = test_only.len();
        for file in &files {
            let gated = runtime
                .iter()
                .any(|(_, parent)| declares(parent, file, true));
            let nested = test_only.iter().any(|parent| {
                declares(
                    &std::fs::read_to_string(dir.join(parent)).unwrap(),
                    file,
                    false,
                )
            });
            if gated || nested {
                test_only.insert(file.clone());
            }
        }
        if test_only.len() == before {
            break;
        }
    }
    for file in &files {
        let runtime_file = runtime.iter().any(|(name, _)| name == file);
        assert!(
            runtime_file || file == "production.rs" || test_only.contains(file),
            "page_host/{file} is runtime code the scan misses"
        );
    }
    for (_, source) in runtime {
        for line in source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
        {
            assert!(!line.contains("page_state::"));
            assert!(
                !line.contains("std::fs") && !line.contains("std::io") && !line.contains("cap_std")
            );
        }
    }
    let adapter = include_str!("production.rs");
    assert!(include_str!("mod.rs").contains("\nmod production;"));
    for line in adapter
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
    {
        for bypass in [
            "fs::write(",
            "fs::rename(",
            "sync_all()",
            "MoveFileExW(",
            "renameat2(",
        ] {
            assert!(
                !line.contains(bypass),
                "adapter bypasses audited primitive: {line}"
            );
        }
    }
}

/// The disk publications (Observed events) since event `from`.
fn observed_since(h: &Host<ModelFs>, from: usize) -> Vec<Text> {
    h.events[from..]
        .iter()
        .filter_map(|event| match event {
            Event::Observed { bytes, .. } => Some(bytes.clone()),
            _ => None,
        })
        .collect()
}

/// V3 (REVIEW-3a), the observation path: an own save of P leaves the last
/// observation at A; reading A back adopts it and publishes it, although
/// it equals that observation. A submit publishes no disk state.
#[test]
fn v3_an_observed_return_to_the_old_observation_publishes_it() {
    let mut h = host();
    open(&mut h, "a.md");
    edit(&mut h, "a.md", "P");
    saved(&mut h, "a.md");
    assert_eq!(h.pages["a.md"].obs, Some(text("A")));
    h.fs.external("a.md", text("A"), true);
    let from = h.events.len();
    assert_eq!(h.observe("a.md"), Disposition::Applied);
    assert_eq!(h.pages["a.md"].buf, text("A"));
    assert_eq!(observed_since(&h, from), vec![text("A")], "V3");
    let from = h.events.len();
    edit(&mut h, "a.md", "Q");
    assert!(observed_since(&h, from).is_empty(), "a submit is no read");
}

/// V3 (REVIEW-3a), the save's mismatch path: after an own save of P the
/// file returns to the old observation A while Q is typed; the next save's
/// guard reads A, conflicts, and publishes A.
#[test]
fn v3_a_save_mismatch_back_to_the_old_observation_publishes_it() {
    let mut h = host();
    open(&mut h, "a.md");
    edit(&mut h, "a.md", "P");
    saved(&mut h, "a.md");
    edit(&mut h, "a.md", "Q");
    h.fs.external("a.md", text("A"), true);
    let from = h.events.len();
    assert_eq!(h.start_save("a.md"), Disposition::Pending);
    for _ in 0..10 {
        if h.job.is_none() {
            break;
        }
        let epoch = h.fs.epochs.get("a.md").copied().unwrap_or(0);
        h.advance_save(epoch);
    }
    assert!(h.pages["a.md"].conflict);
    assert_eq!(
        h.fs.files["graph/a.md"],
        Arc::from(&b"A"[..]),
        "no overwrite"
    );
    assert_eq!(observed_since(&h, from), vec![text("A")], "V3");
}
