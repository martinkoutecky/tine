use super::*;
use crate::page_host::progress::{Clock, Progress};
use std::cell::Cell;

struct ManualClock(Cell<u64>);
impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.0.get()
    }
}
type Timed = Progress<ModelFs, ManualClock>;

fn timed() -> Timed {
    Progress::new(host(), ManualClock(Cell::new(0)))
}
fn time(p: &Timed, now: u64) {
    assert!(now >= p.clock.now_ms());
    p.clock.0.set(now);
}
fn poll(p: &mut Timed) -> Disposition {
    let epoch = p
        .host
        .job
        .as_ref()
        .and_then(|j| p.host.fs.epochs.get(&j.page))
        .copied()
        .unwrap_or(0);
    p.poll(epoch)
}
fn pump(p: &mut Timed) {
    for _ in 0..96 {
        if poll(p) == Disposition::Disabled {
            return;
        }
    }
    panic!("progress did not quiesce on a working disk");
}
fn begin_edit(p: &mut Timed) {
    p.with_host(|h| {
        open(h, "a.md");
        edit(h, "a.md", "first");
    });
}

fn failed_refresh(p: &mut Timed) {
    begin_edit(p);
    p.with_host(|h| {
        h.fs.external("a.md", text("theirs"), true);
        assert_eq!(h.observe("a.md"), Disposition::Applied);
    });
    p.host.fs.inject(Phase::DraftTemp, [Fault::Before]);
    pump(p);
    time(p, 100);
    pump(p);
    assert!(p.host.worker.is_none());
    assert!(p.host.logical_drafts().is_empty());
    assert!(p.notice("a.md").draft_error);
}

#[test]
fn review_r1_failed_move_cannot_clear_unrecovered_source_refresh_error() {
    let mut p = timed();
    p.with_host(|h| open(h, "b.md"));
    failed_refresh(&mut p);
    p.host.fs.inject(Phase::DraftTemp, [Fault::Before]);
    p.with_host(|h| {
        assert_eq!(
            send(
                h,
                "a.md",
                RequestKind::Move {
                    receiver: "b.md".into(),
                    source_text: text("source"),
                    receiver_text: text("receiver"),
                    source_version: h.pages["a.md"].version,
                    receiver_version: h.pages["b.md"].version,
                }
            ),
            Disposition::Pending
        );
    });
    pump(&mut p);
    time(&p, 200);
    pump(&mut p);
    assert!(p.host.worker.is_none());
    assert!(!p.host.receive("a.md").unwrap().answer.unwrap().took);
    assert!(!p.host.receive("b.md").unwrap().answer.unwrap().took);
    assert!(p.host.pages["a.md"].risk);
    assert!(p.host.logical_drafts().is_empty());
    assert!(
        p.notice("a.md").draft_error,
        "a different refused effect cannot recover the refresh"
    );
    assert!(!p.notice("b.md").draft_error);
    time(&p, 500);
    pump(&mut p);
    assert!(!p.notice("a.md").draft_error);
    assert!(p.host.logical_drafts().contains_key("a.md"));
}

#[test]
fn review_r1_discard_clears_obsolete_terminal_refresh_error() {
    let mut p = timed();
    failed_refresh(&mut p);
    p.with_host(|h| {
        assert_eq!(
            send(
                h,
                "a.md",
                RequestKind::Discard {
                    version: h.pages["a.md"].version,
                }
            ),
            Disposition::Applied
        )
    });
    time(&p, 100_000);
    pump(&mut p);
    assert!(p.host.pages["a.md"].clean());
    assert!(!p.host.pages["a.md"].risk);
    assert!(p.host.subscriptions.contains("a.md"));
    assert!(p.host.worker.is_none());
    assert!(
        !p.notice("a.md").draft_error,
        "terminal abandoned refresh has no remaining obligation"
    );
}

#[test]
fn review_r1_refresh_recovery_is_specific_to_its_desired_page() {
    let mut p = timed();
    failed_refresh(&mut p);
    p.with_host(|h| {
        assert_eq!(h.delete("a.md"), Disposition::Refused);
        open(h, "b.md");
        edit(h, "b.md", "b unsaved");
        h.fs.external("b.md", text("b external"), true);
        assert_eq!(h.observe("b.md"), Disposition::Applied);
    });
    assert!(
        p.notice("a.md").draft_error,
        "a refused operation is not recovery"
    );
    p.host.fs.inject(Phase::DraftTemp, [Fault::Before]);
    pump(&mut p);
    time(&p, 200);
    pump(&mut p);
    assert!(p.notice("a.md").draft_error);
    assert!(p.notice("b.md").draft_error);
    time(&p, 500);
    pump(&mut p);
    assert!(!p.notice("a.md").draft_error);
    assert!(
        p.notice("b.md").draft_error,
        "another page's refresh cannot recover b"
    );
    time(&p, 600);
    pump(&mut p);
    assert!(!p.notice("b.md").draft_error);
}

#[test]
fn review_r2_single_retryable_sync_failure_does_not_report_conflict_early() {
    let mut p = timed();
    begin_edit(&mut p);
    p.with_host(|h| {
        h.fs.external("a.md", text("theirs"), true);
        h.observe("a.md");
    });
    for _ in 0..3 {
        poll(&mut p);
    } // start, temp, rename
    p.host.fs.inject(Phase::DraftSync, [Fault::Before]);
    poll(&mut p);
    let worker = p.host.worker.as_ref().unwrap();
    assert_eq!(worker.task.stage, Stage::Sync);
    assert_eq!(worker.task.failures, 1);
    assert!(p.host.logical_drafts().is_empty());
    assert!(!p
        .host
        .events
        .iter()
        .any(|e| matches!(e, Event::DraftError { .. })));
    assert!(!p.notice("a.md").draft_error);
    assert!(!p.notice("a.md").conflict_reported);
    pump(&mut p);
    assert!(p.notice("a.md").conflict_reported);
    assert!(!p.notice("a.md").draft_error);
}

fn capped_draft_backoff() -> Timed {
    let mut p = timed();
    begin_edit(&mut p);
    p.with_host(|h| {
        open(h, "b.md");
        edit(h, "b.md", "healthy b");
        h.fs.external("a.md", text("theirs"), true);
        h.observe("a.md");
    });
    p.host.fs.inject(Phase::DraftSync, [Fault::Before; 100]);
    for _ in 0..40 {
        poll(&mut p);
        if let Some(due) = p.draft_retry_at() {
            if due == 74_400 {
                assert_eq!(p.clock.now_ms(), 44_400);
                assert_eq!(
                    p.host.worker.as_ref().unwrap().task.stage,
                    Stage::CleanupSync
                );
                assert!(p.host.job.is_none());
                assert!(!p.host.busy("b.md"));
                assert!(!p.host.allocator_busy());
                assert_eq!(p.host.fs.files["graph/b.md"].as_ref(), b"B");
                p.host.fs.faults.clear();
                return p;
            }
            time(&p, due);
        }
    }
    panic!("draft retry did not reach capped backoff");
}

#[test]
fn review_r3_backoff_does_not_stall_unrelated_overdue_save() {
    let mut p = capped_draft_backoff();
    let calls = p
        .host
        .fs
        .calls
        .iter()
        .filter(|&&c| c == Phase::DraftSync)
        .count();
    pump(&mut p);
    assert_eq!(
        p.host.fs.files["graph/b.md"].as_ref(),
        b"healthy b",
        "healthy b save is blocked until unrelated draft retry at 74400, now 44400"
    );
    assert!(p.host.pages["b.md"].clean());
    assert!(p.host.worker.is_some());
    assert_eq!(p.draft_retry_at(), Some(74_400));
    assert!(p.host.busy("a.md"));
    assert!(p.notice("a.md").draft_error);
    assert_eq!(
        p.host
            .fs
            .calls
            .iter()
            .filter(|&&c| c == Phase::DraftSync)
            .count(),
        calls
    );
    time(&p, 74_400);
    pump(&mut p);
    assert!(p.host.worker.is_none());
    assert!(!p.notice("a.md").draft_error);
}

#[test]
fn review_r3_backoff_does_not_stall_an_already_running_unrelated_save() {
    let mut p = capped_draft_backoff();
    p.with_host(|h| assert_eq!(h.start_save("b.md"), Disposition::Pending));
    pump(&mut p);
    assert!(
        p.host.job.is_none(),
        "running save must finish during draft backoff"
    );
    assert_eq!(p.host.fs.files["graph/b.md"].as_ref(), b"healthy b");
    assert_eq!(p.draft_retry_at(), Some(74_400));
    assert_eq!(
        p.host.worker.as_ref().unwrap().task.stage,
        Stage::CleanupSync
    );
}

fn applied_delete(p: &mut Timed) {
    assert_eq!(p.with_host(|h| h.delete("a.md")), Disposition::Pending);
    for _ in 0..4 {
        poll(p);
    }
    assert!(p.host.pages["a.md"].buf.is_none());
    assert!(p.host.worker.as_ref().unwrap().retry_copy);
}

#[test]
fn review_f1_explosion_error_survives_logical_match_until_physical_completion() {
    let mut p = timed();
    applied_delete(&mut p);
    p.host.fs.inject(Phase::DraftSync, [Fault::Before; 3]);
    for _ in 0..5 {
        poll(&mut p);
    }
    assert!(p.host.worker.is_some());
    assert!(p
        .host
        .events
        .iter()
        .any(|e| matches!(e, Event::DraftError { .. })));
    assert!(
        p.notice("a.md").draft_error,
        "pending physical failure remains visible"
    );
    p.with_host(|_| ());
    assert!(p.notice("a.md").draft_error);
    time(&p, 100_000);
    pump(&mut p);
    assert!(p.host.worker.is_none());
    assert!(!p.notice("a.md").draft_error);
}

#[test]
fn review_f1_replaced_explosion_vehicles_accumulate_failures() {
    let mut p = timed();
    applied_delete(&mut p);
    p.host.fs.inject(Phase::DraftTemp, [Fault::Before; 30]);
    for n in 1..=30 {
        time(&p, n * 30_000);
        poll(&mut p);
    }
    assert!(p.host.worker.is_some());
    assert!(p.host.allocator_busy());
    assert!(p
        .host
        .events
        .iter()
        .any(|e| matches!(e, Event::DraftError { .. })));
    assert!(p.notice("a.md").draft_error);
    p.host.fs.faults.clear();
    time(&p, 1_000_000);
    pump(&mut p);
    assert!(p.host.worker.is_none());
    assert!(!p.notice("a.md").draft_error);
}

#[test]
fn review_f3_cleanup_sync_backs_off_at_a_fixed_clock() {
    let mut p = timed();
    begin_edit(&mut p);
    p.with_host(|h| {
        h.fs.external("a.md", text("theirs"), true);
        h.observe("a.md");
    });
    p.host.fs.inject(Phase::DraftSync, [Fault::Before; 100]);
    for _ in 0..30 {
        poll(&mut p);
    }
    let syncs = p
        .host
        .fs
        .calls
        .iter()
        .filter(|&&c| c == Phase::DraftSync)
        .count();
    assert_eq!(
        syncs, 3,
        "only the initial sync and two immediate retries at clock zero"
    );
    assert!(p.notice("a.md").draft_error);
}

#[test]
fn review_f4_recurring_first_page_cannot_starve_a_quiet_overdue_page() {
    let mut p = timed();
    p.with_host(|h| {
        open(h, "a.md");
        open(h, "b.md");
        edit(h, "a.md", "recurring a");
        edit(h, "b.md", "waiting b");
    });
    time(&p, 400);
    for n in 0..20 {
        poll(&mut p);
        if p.host.job.as_ref().is_some_and(|j| j.page == "b.md") {
            pump(&mut p);
            break;
        }
        assert_eq!(p.host.job.as_ref().unwrap().page, "a.md");
        for _ in 0..4 {
            time(&p, p.clock.now_ms() + 125);
            poll(&mut p);
        }
        p.with_host(|h| edit(h, "a.md", &format!("recurring {n}")));
        time(&p, p.clock.now_ms() + 400);
    }
    assert_eq!(p.host.fs.files["graph/b.md"].as_ref(), b"waiting b");
}

#[test]
fn review_f5_operation_loaded_page_retires_after_publication_and_draft_removal() {
    let mut p = timed();
    applied_delete(&mut p);
    pump(&mut p);
    time(&p, 400);
    pump(&mut p);
    assert!(p.host.subscriptions.is_empty());
    assert!(p.host.logical_drafts().is_empty());
    assert!(
        !p.host.pages.contains_key("a.md"),
        "clean operation-only slot retires"
    );
}

#[test]
fn review_f3_every_retained_effect_uses_backoff_and_missing_sync_only() {
    for effect in ["cleanup", "representation", "retirement", "op-retirement"] {
        let mut p = timed();
        let phase = if effect == "representation" {
            Phase::DraftTemp
        } else {
            Phase::DraftSync
        };
        match effect {
            "representation" => applied_delete(&mut p),
            "op-retirement" => {
                applied_delete(&mut p);
                for _ in 0..32 {
                    let w = p.host.worker.as_ref().unwrap();
                    if w.task.name.starts_with("op-") && w.task.stage == Stage::UnlinkSync {
                        break;
                    }
                    poll(&mut p);
                }
                assert_eq!(
                    p.host.worker.as_ref().unwrap().task.stage,
                    Stage::UnlinkSync
                );
            }
            _ => {
                begin_edit(&mut p);
                p.with_host(|h| {
                    h.fs.external("a.md", text("theirs"), true);
                    h.observe("a.md");
                });
                if effect == "retirement" {
                    pump(&mut p);
                    p.with_host(|h| {
                        edit(h, "a.md", "newer");
                        assert_eq!(h.begin_draft("a.md"), Disposition::Pending);
                    });
                    for _ in 0..5 {
                        poll(&mut p);
                    }
                    assert_eq!(
                        p.host.worker.as_ref().unwrap().task.stage,
                        Stage::UnlinkSync
                    );
                } else {
                    poll(&mut p); // admission
                    poll(&mut p); // temp
                    poll(&mut p); // rename
                    p.host.fs.inject(Phase::DraftSync, [Fault::Before; 3]);
                    for _ in 0..3 {
                        poll(&mut p);
                    }
                    assert_eq!(p.draft_retry_at(), Some(100));
                    time(&p, 100);
                    poll(&mut p); // completed cleanup unlink
                    assert_eq!(
                        p.host.worker.as_ref().unwrap().task.stage,
                        Stage::CleanupSync
                    );
                }
            }
        }
        let unlinks = p
            .host
            .fs
            .calls
            .iter()
            .filter(|&&c| c == Phase::DraftUnlink)
            .count();
        p.host.fs.inject(phase, [Fault::Before; 8]);
        let delays = if effect == "cleanup" {
            vec![300, 1000, 3000, 10000, 30000, 30000]
        } else {
            vec![100, 300, 1000, 3000, 10000, 30000, 30000]
        };
        for delay in delays {
            // A failed replacement write has a separate terminal/replacement barrier.
            for _ in 0..16 {
                if p.draft_retry_at().is_some() {
                    break;
                }
                poll(&mut p);
            }
            let due = p
                .draft_retry_at()
                .expect("failed effect must schedule its retry");
            assert_eq!(due, p.clock.now_ms() + delay, "{effect}");
            let calls = p.host.fs.calls.len();
            for _ in 0..8 {
                assert_eq!(poll(&mut p), Disposition::Disabled);
            }
            assert_eq!(
                p.host.fs.calls.len(),
                calls,
                "{effect} early polls must not touch disk"
            );
            time(&p, due - 1);
            assert_eq!(poll(&mut p), Disposition::Disabled);
            time(&p, due);
            poll(&mut p);
        }
        assert!(p.notice("a.md").draft_error);
        if effect != "representation" {
            assert_eq!(
                p.host
                    .fs
                    .calls
                    .iter()
                    .filter(|&&c| c == Phase::DraftUnlink)
                    .count(),
                unlinks,
                "{effect} must never repeat a completed unlink"
            );
        }
        p.host.fs.faults.clear();
        if let Some(due) = p.draft_retry_at() {
            time(&p, due);
        }
        pump(&mut p);
        assert!(p.host.worker.is_none(), "{effect} recovers");
        assert!(
            !p.notice("a.md").draft_error,
            "{effect} clears only after recovery"
        );
        assert!(!p.host.allocator_busy());
    }
}

#[test]
fn review_f5_retirement_preserves_subscriptions_and_queued_open_custody() {
    let mut p = timed();
    p.with_host(|h| {
        open(h, "a.md");
    });
    pump(&mut p);
    assert!(p.host.pages["a.md"].clean());
    assert!(p.host.subscriptions.contains("a.md"));

    let mut p = timed();
    p.with_host(|h| {
        assert_eq!(h.load("a.md"), Disposition::Applied);
        assert_eq!(
            h.admit(Request {
                id: 1,
                generation: h.generation,
                page: "a.md".into(),
                kind: RequestKind::Open
            }),
            Disposition::Applied
        );
        h.window_crash(); // queued open retains custody even without a subscription
    });
    assert_eq!(poll(&mut p), Disposition::Pending);
    assert!(p.host.pages.contains_key("a.md"));
    assert!(p.host.applying.is_some());
    assert_eq!(poll(&mut p), Disposition::Applied);
    assert!(p.host.pages.contains_key("a.md"));
    assert_eq!(poll(&mut p), Disposition::Applied);
    assert!(!p.host.pages.contains_key("a.md"));
}

#[test]
fn quiet_edit_and_debounce_publish_at_400_ms_after_last_change() {
    for second in [false, true] {
        let mut p = timed();
        begin_edit(&mut p);
        if second {
            time(&p, 300);
            p.with_host(|h| edit(h, "a.md", "last"));
        }
        let due = if second { 700 } else { 400 };
        time(&p, due - 1);
        pump(&mut p);
        assert!(p.host.job.is_none());
        assert!(!p.host.pages["a.md"].clean());
        time(&p, due);
        pump(&mut p);
        assert!(p.host.pages["a.md"].clean());
        assert_eq!(
            p.host.fs.files["graph/a.md"].as_ref(),
            if second { &b"last"[..] } else { &b"first"[..] }
        );
    }
}

#[test]
fn accepted_same_bytes_with_a_new_version_restart_the_debounce() {
    let mut p = timed();
    begin_edit(&mut p);
    time(&p, 300);
    p.with_host(|h| edit(h, "a.md", "first"));
    time(&p, 699);
    pump(&mut p);
    assert!(!p.host.pages["a.md"].clean());
    time(&p, 700);
    pump(&mut p);
    assert!(p.host.pages["a.md"].clean());
}

#[test]
fn overdue_save_starts_before_queued_requests_can_starve_it() {
    let mut p = timed();
    begin_edit(&mut p);
    for _ in 0..8 {
        p.with_host(|h| {
            assert_eq!(
                h.admit(Request {
                    id: h.last_admitted + 1,
                    generation: h.generation,
                    page: "a.md".into(),
                    kind: RequestKind::Open
                }),
                Disposition::Applied
            );
            h.window_crash();
        });
    }
    time(&p, 1000);
    assert_eq!(poll(&mut p), Disposition::Pending);
    assert!(
        p.host.job.is_some(),
        "deadline takes priority over more queue work"
    );
    assert_eq!(p.host.queue.len(), 8);
    pump(&mut p);
    assert_eq!(p.host.fs.files["graph/a.md"].as_ref(), b"first");
    assert!(!p.host.pages.contains_key("a.md"));
    assert!(p.host.abstract_queue().is_empty());
}

#[test]
fn a_publication_followed_by_new_input_in_one_boundary_resets_unsaved_age() {
    let mut p = timed();
    begin_edit(&mut p);
    time(&p, 2000);
    p.with_host(|h| {
        saved(h, "a.md");
        edit(h, "a.md", "new unsaved input");
    });
    time(&p, 2100);
    pump(&mut p);
    assert!(!p.host.pages["a.md"].clean());
    time(&p, 2399);
    pump(&mut p);
    assert!(!p.host.pages["a.md"].clean());
    time(&p, 2400);
    pump(&mut p);
    assert!(p.host.pages["a.md"].clean());
}

#[test]
fn continuous_edits_publish_at_the_one_second_cap() {
    let mut p = timed();
    begin_edit(&mut p);
    for (now, bytes) in [(300, "second"), (600, "third"), (900, "last")] {
        time(&p, now);
        p.with_host(|h| edit(h, "a.md", bytes));
    }
    time(&p, 999);
    pump(&mut p);
    assert!(p.host.job.is_none());
    time(&p, 1000);
    pump(&mut p);
    assert!(p.host.pages["a.md"].clean());
    assert_eq!(p.host.fs.files["graph/a.md"].as_ref(), b"last");
}

#[test]
fn failed_and_uncertain_saves_retry_on_every_backoff_and_surface_persistent_error() {
    for phase in [Phase::PageTemp, Phase::PageSync] {
        let mut p = timed();
        begin_edit(&mut p);
        let mut due = 400;
        for (index, delay) in [100, 300, 1000, 3000, 10000, 30000, 30000]
            .into_iter()
            .enumerate()
        {
            time(&p, due - 1);
            pump(&mut p);
            assert!(p.host.job.is_none());
            p.host.fs.inject(phase, [Fault::Before]);
            time(&p, due);
            pump(&mut p);
            let notice = p.notice("a.md");
            assert_eq!(notice.failures, index as u32 + 1);
            assert_eq!(notice.save_error, index >= 2);
            assert!(p.host.pages["a.md"].risk);
            assert!(p.host.logical_drafts().contains_key("a.md"));
            if index == 0 {
                assert_eq!(
                    p.host
                        .fs
                        .calls
                        .iter()
                        .filter(|p| **p == Phase::DraftTemp)
                        .count(),
                    1
                );
            }
            due += delay;
        }
        time(&p, due - 1);
        pump(&mut p);
        assert!(p.notice("a.md").save_error);
        time(&p, due);
        pump(&mut p);
        assert!(p.host.pages["a.md"].clean());
        assert_eq!(p.notice("a.md").failures, 0);
        assert!(!p.notice("a.md").save_error);
        assert!(p.host.logical_drafts().is_empty());
    }
}

#[test]
fn conflict_indicator_waits_for_applied_draft_or_visible_draft_failure() {
    for failing in [false, true] {
        let mut p = timed();
        begin_edit(&mut p);
        p.with_host(|h| {
            h.fs.external("a.md", text("theirs"), true);
            assert_eq!(h.observe("a.md"), Disposition::Applied);
        });
        assert!(p.host.outbox["a.md"].page.as_ref().unwrap().conflict);
        assert!(!p.notice("a.md").conflict_reported);
        if failing {
            p.host.fs.inject(Phase::DraftTemp, [Fault::Before]);
        }
        assert_eq!(poll(&mut p), Disposition::Pending);
        assert!(!p.notice("a.md").conflict_reported);
        if !failing {
            for _ in 0..3 {
                poll(&mut p);
                assert!(!p.notice("a.md").conflict_reported);
                assert!(!p.notice("a.md").draft_error);
            }
        }
        pump(&mut p);
        assert!(p.notice("a.md").conflict_reported);
        assert_eq!(p.notice("a.md").draft_error, failing);
        assert_eq!(p.host.logical_drafts().contains_key("a.md"), !failing);
        assert!(p.host.job.is_none());
    }
}

#[test]
fn at_risk_refreshes_coalesce_for_500_ms_and_capture_the_latest_edit() {
    let mut p = timed();
    begin_edit(&mut p);
    p.with_host(|h| {
        h.fs.external("a.md", text("theirs"), true);
        h.observe("a.md");
    });
    pump(&mut p);
    for (now, bytes) in [(100, "second"), (200, "last")] {
        time(&p, now);
        p.with_host(|h| edit(h, "a.md", bytes));
        pump(&mut p);
        assert!(p.notice("a.md").conflict_reported);
    }
    time(&p, 499);
    pump(&mut p);
    assert_eq!(
        p.host
            .fs
            .calls
            .iter()
            .filter(|p| **p == Phase::DraftTemp)
            .count(),
        1
    );
    time(&p, 500);
    pump(&mut p);
    assert_eq!(
        p.host
            .fs
            .calls
            .iter()
            .filter(|p| **p == Phase::DraftTemp)
            .count(),
        2
    );
    assert_eq!(p.host.logical_drafts()["a.md"].bytes, text("last"));
    p.with_host(|h| {
        assert_eq!(
            send(
                h,
                "a.md",
                RequestKind::Submit {
                    bytes: text("resolved"),
                    version: h.pages["a.md"].version,
                    resolve: Some(text("theirs"))
                }
            ),
            Disposition::Applied
        )
    });
    assert!(!p.notice("a.md").conflict_reported);
}

#[test]
fn refresh_cooldown_is_measured_from_the_actual_temp_write() {
    let mut p = timed();
    begin_edit(&mut p);
    p.with_host(|h| {
        h.fs.external("a.md", text("theirs"), true);
        h.observe("a.md");
    });
    assert_eq!(poll(&mut p), Disposition::Pending);
    time(&p, 1000);
    poll(&mut p); // temp was delayed after admission
    time(&p, 1100);
    poll(&mut p);
    time(&p, 1200);
    poll(&mut p);
    time(&p, 1300);
    poll(&mut p);
    assert!(p.host.worker.is_none());
    p.with_host(|h| edit(h, "a.md", "latest"));
    time(&p, 1499);
    pump(&mut p);
    assert_eq!(
        p.host
            .fs
            .calls
            .iter()
            .filter(|p| **p == Phase::DraftTemp)
            .count(),
        1
    );
    time(&p, 1500);
    pump(&mut p);
    assert_eq!(
        p.host
            .fs
            .calls
            .iter()
            .filter(|p| **p == Phase::DraftTemp)
            .count(),
        2
    );
    assert_eq!(p.host.logical_drafts()["a.md"].bytes, text("latest"));
}

#[test]
fn stale_same_bytes_draft_does_not_report_a_new_conflict_before_refresh() {
    for changed_base in [false, true] {
        let mut p = timed();
        begin_edit(&mut p);
        p.host.fs.inject(Phase::PageTemp, [Fault::Before]);
        time(&p, 400);
        pump(&mut p);
        time(&p, 450);
        if changed_base {
            p.with_host(|h| {
                h.fs.external("a.md", text("first"), true);
                h.observe("a.md");
            });
        } else {
            p.with_host(|h| {
                edit(h, "a.md", "different");
                edit(h, "a.md", "first");
            });
        }
        p.with_host(|h| {
            h.fs.external("a.md", text("theirs"), true);
            h.observe("a.md");
        });
        assert!(!p.notice("a.md").conflict_reported);
        time(&p, 899);
        pump(&mut p);
        assert!(!p.notice("a.md").conflict_reported);
        time(&p, 900);
        pump(&mut p);
        assert!(p.notice("a.md").conflict_reported);
        let record = &p.host.logical_drafts()["a.md"];
        assert_eq!(record.version, p.host.pages["a.md"].version);
        assert_eq!(record.base, p.host.pages["a.md"].base);
    }
}

#[test]
fn pending_move_keeps_custody_and_answers_after_disk_recovery() {
    let mut p = timed();
    p.with_host(|h| {
        open(h, "a.md");
        open(h, "b.md");
    });
    p.host.fs.inject(Phase::DraftSync, [Fault::Before; 20]);
    p.with_host(|h| {
        let source_version = h.pages["a.md"].version;
        let receiver_version = h.pages["b.md"].version;
        assert_eq!(
            send(
                h,
                "a.md",
                RequestKind::Move {
                    receiver: "b.md".into(),
                    source_text: text("source"),
                    receiver_text: text("target"),
                    source_version,
                    receiver_version
                }
            ),
            Disposition::Pending
        );
    });
    for now in 1..=15 {
        time(&p, now * 100);
        poll(&mut p);
    }
    assert!(p.host.worker.is_some());
    assert!(p.host.applying.is_some());
    assert!(p.host.receive("a.md").is_none());
    assert!(p.host.allocator_busy());
    assert!(p.notice("b.md").draft_error);
    p.with_host(|_| ());
    assert!(
        p.notice("b.md").draft_error,
        "pending effect error stays visible between polls"
    );
    p.host.fs.faults.clear();
    time(&p, p.draft_retry_at().unwrap());
    pump(&mut p);
    assert!(p.host.worker.is_none());
    assert!(p.host.applying.is_none());
    assert!(!p.host.allocator_busy());
    assert!(p.host.lock_ownership.is_empty());
    for key in ["a.md", "b.md"] {
        assert!(!p.host.receive(key).unwrap().answer.unwrap().took);
    }
    assert_eq!(p.host.pages["a.md"].buf, text("A"));
    assert_eq!(p.host.pages["b.md"].buf, text("B"));
    p.with_host(|h| edit(h, "a.md", "healthy retry"));
    time(&p, p.clock.now_ms() + 400);
    pump(&mut p);
    assert!(!p.notice("a.md").draft_error);
}

#[test]
fn applied_group_operation_retries_explosion_and_retires_every_vehicle() {
    let mut p = timed();
    p.with_host(|h| {
        assert_eq!(
            h.rename_with(
                "a.md",
                "c.md",
                &BTreeSet::from(["b.md".into()]),
                |bytes, _, _| Ok(bytes.clone())
            ),
            Disposition::Pending
        )
    });
    for _ in 0..4 {
        poll(&mut p);
    }
    assert_eq!(p.host.pages["c.md"].buf, text("A"));
    assert!(p.host.worker.is_some());
    p.host.fs.inject(Phase::DraftTemp, [Fault::Before]);
    pump(&mut p);
    assert!(p.host.worker.is_some());
    time(&p, p.draft_retry_at().unwrap());
    pump(&mut p);
    assert!(p.host.worker.is_none());
    assert!(!p.host.allocator_busy());
    let scan = drafts::scan(p.host.fs.draft_files(true));
    assert_eq!(scan.logical.len(), 3);
    assert!(scan.files.keys().all(|name| name.starts_with("p-")));
    assert_eq!(scan.files.len(), 3);
}

#[test]
fn queued_requests_dequeue_apply_answer_and_then_save_on_the_clock() {
    let mut p = timed();
    p.with_host(|h| {
        assert_eq!(
            h.admit(Request {
                id: 1,
                generation: h.generation,
                page: "a.md".into(),
                kind: RequestKind::Open
            }),
            Disposition::Applied
        )
    });
    assert_eq!(poll(&mut p), Disposition::Pending);
    assert!(p.host.applying.is_some());
    assert_eq!(p.host.abstract_queue().len(), 1);
    assert_eq!(poll(&mut p), Disposition::Applied);
    assert!(p.host.receive("a.md").unwrap().answer.is_some());
    p.with_host(|h| {
        assert_eq!(
            h.admit(Request {
                id: 2,
                generation: h.generation,
                page: "a.md".into(),
                kind: RequestKind::Submit {
                    bytes: text("queued"),
                    version: h.pages["a.md"].version,
                    resolve: None
                }
            }),
            Disposition::Applied
        )
    });
    pump(&mut p);
    assert!(p.host.receive("a.md").unwrap().answer.unwrap().took);
    assert_eq!(p.host.last_applied, p.host.last_admitted);
    time(&p, 399);
    pump(&mut p);
    assert!(!p.host.pages["a.md"].clean());
    time(&p, 400);
    pump(&mut p);
    assert!(p.host.pages["a.md"].clean());
}

/// Polls every 100 ms of clock time for `ms`.
fn run_for(p: &mut Timed, ms: u64) {
    let end = p.clock.now_ms() + ms;
    while p.clock.now_ms() < end {
        time(p, p.clock.now_ms() + 100);
        poll(p);
    }
}

/// Deletes A through its move, then fails the deletion's own custody (a
/// reported error), leaving the marker as debt. Returns its payload.
fn moved_with_unfinished_custody(h: &mut Host<ModelFs>) -> String {
    open(h, "a.md");
    assert_eq!(h.delete("a.md"), Disposition::Pending);
    drain(h);
    assert_eq!(h.start_save("a.md"), Disposition::Pending);
    for _ in 0..3 {
        h.advance_save(0); // guard, marker, move
    }
    assert_eq!(h.job.as_ref().unwrap().phase, SavePhase::TrashSync);
    h.fs.inject(Phase::TrashSync, [Fault::Before]);
    h.advance_save(0);
    assert!(h.job.is_none());
    let debt = &h.custody["a.md"];
    debt.markers.values().next().unwrap().clone()
}

fn custody_entries(p: &Timed) -> usize {
    let custody = |k: &&String| k.starts_with("draft/trash-custody/");
    p.host.fs.files.keys().filter(custody).count()
}

/// REVIEW-2b-r2 V1: unknown custody is relisted only on an idle host. A
/// deletion in flight has published its marker but not moved yet; adopting
/// that marker would settle it (nothing moved yet) and retire it before the
/// move, leaving the moved payload without custody.
#[test]
fn r2_unknown_custody_is_not_relisted_under_a_running_deletion() {
    let mut h = host();
    h.stop();
    h.fs.inject(Phase::CustodyList, [Fault::Before]);
    assert_eq!(h.launch(), Disposition::Applied);
    assert!(h.custody_unknown.is_some());
    open(&mut h, "a.md");
    assert_eq!(h.delete("a.md"), Disposition::Pending);
    drain(&mut h);
    assert_eq!(h.start_save("a.md"), Disposition::Pending);
    h.advance_save(0); // guard
    h.advance_save(0); // marker
    assert_eq!(h.job.as_ref().unwrap().phase, SavePhase::Rename);
    h.recover_custody();
    let custody = |k: &&String| k.starts_with("draft/trash-custody/");
    assert_eq!(h.fs.files.keys().filter(custody).count(), 1);
    assert!(h.custody_unknown.is_some(), "not relisted while busy");
}

/// REVIEW-2b-r2 V3: the escape's custody error is a sticky notice naming the
/// payload. It survives later successful saves and Discard, and clears only
/// when that marker retires.
#[test]
fn r2_escaped_custody_has_a_sticky_progress_notice() {
    let mut h = host();
    let payload = moved_with_unfinished_custody(&mut h);
    assert_eq!(h.observe("a.md"), Disposition::Applied);
    edit(&mut h, "a.md", "C");
    let mut p = Progress::new(h, ManualClock(Cell::new(0)));
    p.host.fs.inject(Phase::TrashSync, [Fault::Before; 64]);
    run_for(&mut p, 10_000);
    assert_eq!(p.host.fs.files["graph/a.md"].as_ref(), b"C");
    let sticky = BTreeSet::from([payload]);
    assert_eq!(p.notice("a.md").custody_error, sticky, "after publication");
    p.with_host(|h| edit(h, "a.md", "D"));
    run_for(&mut p, 10_000);
    assert_eq!(p.host.fs.files["graph/a.md"].as_ref(), b"D");
    assert_eq!(p.notice("a.md").custody_error, sticky, "after a later save");
    p.with_host(|h| {
        edit(h, "a.md", "E");
        let version = h.pages["a.md"].version;
        send(h, "a.md", RequestKind::Discard { version });
    });
    run_for(&mut p, 10_000);
    assert_eq!(p.notice("a.md").custody_error, sticky, "after Discard");
    p.host.fs.faults.clear();
    p.with_host(|h| edit(h, "a.md", "F"));
    run_for(&mut p, 10_000);
    assert_eq!(p.host.fs.files["graph/a.md"].as_ref(), b"F");
    assert!(p.notice("a.md").custody_error.is_empty(), "retired");
    // V3 clears the error at retirement; it does not linger out of sight.
    assert!(
        p.host.custody_errors.is_empty(),
        "{:?}",
        p.host.custody_errors
    );
    assert_eq!(custody_entries(&p), 0);
}

/// REVIEW-2b-r2 V2/V3: retire-only debt never blocks a save; progress retries
/// it with backoff, the third reported failure is a sticky notice naming the
/// payload, and a later successful retirement clears it.
#[test]
fn r2_third_failed_retirement_is_sticky_until_retired() {
    let mut h = host();
    open(&mut h, "a.md");
    assert_eq!(h.delete("a.md"), Disposition::Pending);
    drain(&mut h);
    h.fs.inject(Phase::CustodyRetire, [Fault::Before; 3]);
    assert_eq!(h.start_save("a.md"), Disposition::Pending);
    while h.job.is_some() {
        h.advance_save(0);
    }
    assert!(h.custody.is_empty());
    let payload = h.retire.values().next().unwrap().payload.clone();
    let mut p = Progress::new(h, ManualClock(Cell::new(0)));
    assert!(p.notice("a.md").custody_error.is_empty(), "one failure");
    p.with_host(|h| edit(h, "a.md", "C"));
    let failing = |p: &Timed| p.host.fs.faults[&Phase::CustodyRetire].len();
    for _ in 0..200 {
        if failing(&p) == 0 {
            break;
        }
        assert!(p.notice("a.md").custody_error.is_empty(), "under three");
        run_for(&mut p, 100);
    }
    assert_eq!(failing(&p), 0, "progress retries the retirement");
    let sticky = BTreeSet::from([payload]);
    assert_eq!(p.notice("a.md").custody_error, sticky, "third failure");
    assert_eq!(custody_entries(&p), 1);
    run_for(&mut p, 1_000);
    assert_eq!(p.host.fs.files["graph/a.md"].as_ref(), b"C", "not blocked");
    run_for(&mut p, 10_000);
    assert!(p.notice("a.md").custody_error.is_empty());
    assert_eq!(custody_entries(&p), 0);
}

/// REVIEW-2b-r2 V1: a failed launch listing opens the graph with a sticky
/// custody-unknown condition; polls retry it with backoff, and the first
/// successful listing adopts and settles the debt, then clears it.
#[test]
fn r2_unknown_custody_is_retried_on_the_backoff_until_listed() {
    let mut h = host();
    moved_with_unfinished_custody(&mut h);
    h.stop();
    h.fs.crash();
    h.fs.inject(Phase::CustodyList, [Fault::Before; 3]);
    assert_eq!(h.launch(), Disposition::Applied);
    let mut p = Progress::new(h, ManualClock(Cell::new(0)));
    assert!(p.custody_unknown().unwrap().contains("trash-custody"));
    assert!(p.host.custody.is_empty());
    let mut lists = vec![];
    for _ in 0..200 {
        let before = p.host.fs.calls.len();
        run_for(&mut p, 100);
        if p.host.fs.calls[before..].contains(&Phase::CustodyList) {
            lists.push(p.clock.now_ms());
        }
        if p.custody_unknown().is_none() {
            break;
        }
    }
    assert_eq!(
        lists.len(),
        3,
        "two failed retries, then success: {lists:?}"
    );
    assert!(lists[1] - lists[0] > lists[0], "backoff grows: {lists:?}");
    assert!(p.host.custody.is_empty(), "adopted and settled");
    assert_eq!(custody_entries(&p), 0);
}
