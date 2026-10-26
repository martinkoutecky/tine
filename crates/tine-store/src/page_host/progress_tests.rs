use super::*;
use crate::page_host::binding::{stop_state, Book, StopState};
use crate::page_host::progress::{Clock, Progress, Stopping};
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
    p.poll_at(epoch)
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
    p.with_host(|h| assert_eq!(send(h, "a.md", RequestKind::Discard), Disposition::Applied));
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
    // RENAME-COST L1: no explosion follows the installation.
    assert!(p.host.worker.is_none());
}

/// A crash leaves an applied deletion's operation vehicle; launch explodes
/// it, a representation worker over a.md. Polled to its first copy.
fn launch_exploding(p: &mut Timed) {
    applied_delete(p);
    p.with_host(|h| {
        h.stop();
        h.fs.crash();
        assert_eq!(h.launch(), Disposition::Pending);
    });
    assert!(p.host.worker.as_ref().unwrap().retry_copy);
}

/// RENAME-COST L2: an applied group operation (a.md → c.md, b.md a
/// referrer) whose page b.md is edited: the refresh retires the operation
/// vehicle after durable copies of a's and c's records. Polled to the first
/// copy, a representation phase of the refresh's effect on b.md.
fn absorbing(p: &mut Timed) {
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
        poll(p);
    }
    assert!(p.host.worker.is_none(), "L1: no explosion");
    p.with_host(|h| edit(h, "b.md", "edited b"));
    for _ in 0..16 {
        if p.host.worker.as_ref().is_some_and(|w| w.retry_copy) {
            return;
        }
        poll(p);
    }
    panic!("the refresh never reached its copies");
}

#[test]
fn review_f1_explosion_error_survives_logical_match_until_physical_completion() {
    let mut p = timed();
    absorbing(&mut p);
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
        p.notice("b.md").draft_error,
        "pending physical failure remains visible"
    );
    p.with_host(|_| ());
    assert!(p.notice("b.md").draft_error);
    time(&p, 100_000);
    pump(&mut p);
    assert!(p.host.worker.is_none());
    assert!(!p.notice("b.md").draft_error);
}

#[test]
fn review_f1_replaced_explosion_vehicles_accumulate_failures() {
    let mut p = timed();
    absorbing(&mut p);
    p.host.fs.inject(Phase::DraftTemp, [Fault::Before; 30]);
    for n in 1..=30 {
        time(&p, n * 30_000);
        poll(&mut p);
    }
    assert!(p.host.worker.is_some());
    assert!(p
        .host
        .events
        .iter()
        .any(|e| matches!(e, Event::DraftError { .. })));
    assert!(p.notice("b.md").draft_error);
    p.host.fs.faults.clear();
    time(&p, 1_000_000);
    pump(&mut p);
    assert!(p.host.worker.is_none());
    assert!(!p.notice("b.md").draft_error);
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
            "representation" => launch_exploding(&mut p),
            "op-retirement" => {
                // RENAME-COST L1: retired whole once its page publishes.
                applied_delete(&mut p);
                for _ in 0..32 {
                    let w = p.host.worker.as_ref();
                    if w.is_some_and(|w| {
                        w.task.name.starts_with("op-") && w.task.stage == Stage::UnlinkSync
                    }) {
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

/// A window Move of `a.md` into `b.md` with these texts, at their versions,
/// its operation draft written (the clock does not move).
fn window_move(p: &mut Timed, source: &str, receiver: &str) {
    p.with_host(|h| {
        let (source_version, receiver_version) = (h.pages["a.md"].version, h.pages["b.md"].version);
        assert_eq!(
            send(
                h,
                "a.md",
                RequestKind::Move {
                    receiver: "b.md".into(),
                    source_text: text(source),
                    receiver_text: text(receiver),
                    source_version,
                    receiver_version,
                }
            ),
            Disposition::Pending
        );
    });
    explode(p);
}

/// The host's edit debounce governs a window Move (PERF-FIXES D1): its
/// pages publish 400 ms after their last change, here a second Move at
/// 300 ms, with the same bytes or new ones: a new version restarts it.
#[test]
fn a_window_moves_pages_publish_400_ms_after_their_last_change() {
    for second in [None, Some("source"), Some("again")] {
        let mut p = timed();
        p.with_host(|h| {
            open(h, "a.md");
            open(h, "b.md");
        });
        window_move(&mut p, "source", "receiver");
        if let Some(again) = second {
            time(&p, 300);
            pump(&mut p);
            assert!(
                !p.host.pages["a.md"].clean(),
                "nothing saved before the debounce"
            );
            window_move(&mut p, again, "receiver");
        }
        let due = if second.is_some() { 700 } else { 400 };
        time(&p, due - 1);
        pump(&mut p);
        assert!(!p.host.pages["a.md"].clean() && !p.host.pages["b.md"].clean());
        time(&p, due);
        pump(&mut p);
        assert!(p.host.pages["a.md"].clean() && p.host.pages["b.md"].clean());
        assert_eq!(
            p.host.fs.files["graph/a.md"].as_ref(),
            second.unwrap_or("source").as_bytes()
        );
    }
}

/// A window Submit saves at once (PERF-FIXES D1): the window already waited
/// for the end of the typing burst, so the host adds no second wait. A
/// second Submit after the first one's save saves at once too.
#[test]
fn a_window_submit_saves_at_once_without_the_host_debounce() {
    let mut p = timed();
    begin_edit(&mut p);
    pump(&mut p);
    assert!(p.host.pages["a.md"].clean());
    assert_eq!(p.host.fs.files["graph/a.md"].as_ref(), &b"first"[..]);
    time(&p, 300);
    p.with_host(|h| edit(h, "a.md", "last"));
    pump(&mut p);
    assert!(p.host.pages["a.md"].clean());
    assert_eq!(p.host.fs.files["graph/a.md"].as_ref(), &b"last"[..]);
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
    p.host.test.submits_wait = true;
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
        p.host.test.submits_wait = true;
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
fn applied_group_operation_retires_its_vehicle_whole_after_publication() {
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
    // RENAME-COST L1: no explosion; the operation's pages save at once
    // (caller urgency) while the vehicle alone holds their records.
    assert!(p.host.worker.is_none());
    assert!(!p.host.allocator_busy());
    let files = p.host.fs.draft_files(true);
    assert!(files.len() == 1 && files[0].0.starts_with("op-"));
    pump(&mut p);
    assert!(
        p.host.fs.draft_files(true).is_empty(),
        "the vehicle retires whole"
    );
    let calls = |phase| p.host.fs.calls.iter().filter(|&&c| c == phase).count();
    assert_eq!(calls(Phase::DraftTemp), 1, "one draft file: the vehicle");
    assert_eq!(calls(Phase::DraftUnlink), 1, "retired by one unlink");
}

#[test]
fn queued_requests_dequeue_apply_answer_and_then_save_on_the_clock() {
    let mut p = timed();
    p.host.test.submits_wait = true;
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
        h.advance_save(); // guard, marker, move
    }
    assert_eq!(h.job.as_ref().unwrap().phase, SavePhase::TrashSync);
    h.fs.inject(Phase::TrashSync, [Fault::Before]);
    h.advance_save();
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
    h.advance_save(); // guard
    h.advance_save(); // marker
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
        send(h, "a.md", RequestKind::Discard);
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
        h.advance_save();
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
    // Pending: launch explodes the deletion's surviving operation vehicle.
    assert!(matches!(
        h.launch(),
        Disposition::Applied | Disposition::Pending
    ));
    let mut p = Progress::new(h, ManualClock(Cell::new(0)));
    explode(&mut p);
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

// ---- STEP3 §6/§7: switch and restore stops ----

/// Close admission at the window's last answer and begin a stop.
fn begin_stop(p: &mut Timed, restore: bool) {
    p.with_host(|h| assert_eq!(h.switch_ready(h.last_applied), Disposition::Applied));
    p.stopping = Some(Stopping {
        restore,
        failed: BTreeSet::new(),
    });
}

fn first(p: &Timed, phase: Phase) -> Option<usize> {
    p.host.fs.calls.iter().position(|c| *c == phase)
}

fn state(p: &Timed) -> StopState {
    stop_state(p, &Book::default())
}

/// §6 step 3: a stop saves a dirty page at once (inside its debounce) and
/// never drafts it when that save succeeds.
#[test]
fn a_switch_saves_a_dirty_page_at_once_before_any_draft_of_it() {
    let mut p = timed();
    begin_edit(&mut p);
    begin_stop(&mut p, false);
    p.host.fs.calls.clear();
    pump(&mut p);
    assert!(first(&p, Phase::PageTemp).is_some(), "the save ran at once");
    assert_eq!(
        first(&p, Phase::DraftTemp),
        None,
        "no draft before the save"
    );
    assert!(p.host.pages["a.md"].clean());
    assert_eq!(state(&p), StopState::Ready);
}

/// §6 step 3: a page whose stop save fails is drafted then, and the switch
/// completes over the draft.
#[test]
fn a_switch_drafts_a_page_only_after_its_save_fails() {
    let mut p = timed();
    begin_edit(&mut p);
    begin_stop(&mut p, false);
    p.host.fs.calls.clear();
    p.host.fs.inject(Phase::PageTemp, [Fault::Before]);
    pump(&mut p);
    let (save, draft) = (first(&p, Phase::PageTemp), first(&p, Phase::DraftTemp));
    assert!(
        save.is_some() && draft.is_some() && save < draft,
        "{:?}",
        p.host.fs.calls
    );
    assert!(p.host.pages["a.md"].risk);
    assert_eq!(state(&p), StopState::Ready);
}

/// §6 step 3: a page already at risk (an earlier failed save, drafted) and
/// typed on is saved before its draft is refreshed.
#[test]
fn a_switch_saves_an_at_risk_page_before_refreshing_its_draft() {
    let mut p = timed();
    begin_edit(&mut p);
    p.with_host(|h| {
        risk(h, "a.md");
        draft(h, "a.md");
        edit(h, "a.md", "second");
    });
    begin_stop(&mut p, false);
    time(&p, 1000);
    p.host.fs.calls.clear();
    pump(&mut p);
    assert!(first(&p, Phase::PageTemp).is_some(), "the save ran");
    assert_eq!(first(&p, Phase::DraftTemp), None, "no refresh before it");
    assert!(p.host.pages["a.md"].clean());
    assert_eq!(state(&p), StopState::Ready);
}

/// §6 step 3: a page that cannot be saved (conflict) is drafted at once and
/// never blocks the switch.
#[test]
fn a_switch_drafts_a_conflicted_page_at_once() {
    let mut p = timed();
    begin_edit(&mut p);
    p.with_host(|h| {
        h.fs.external("a.md", text("theirs"), true);
        assert_eq!(h.observe("a.md"), Disposition::Applied);
    });
    begin_stop(&mut p, false);
    p.host.fs.calls.clear();
    pump(&mut p);
    assert_eq!(first(&p, Phase::PageTemp), None);
    assert!(p.host.pages["a.md"].conflict);
    assert_eq!(state(&p), StopState::Ready);
}

/// §6 step 5: a draft that cannot reach a terminal state aborts the switch
/// with the affected page; aborting reopens admission.
#[test]
fn a_switch_aborts_on_a_draft_failure() {
    let mut p = timed();
    begin_edit(&mut p);
    begin_stop(&mut p, false);
    p.host.fs.inject(Phase::PageTemp, [Fault::Before; 10]);
    p.host.fs.inject(Phase::DraftTemp, [Fault::Before; 10]);
    for now in [0, 100, 400, 1400, 4400] {
        time(&p, now);
        pump(&mut p);
    }
    assert_eq!(
        state(&p),
        StopState::Aborted(BTreeSet::from(["a.md".into()]))
    );
    assert!(!p.host.can_switch());
    p.with_host(|h| h.switch_abort());
    assert!(p.stopping.is_none());
    assert!(p.host.admission_open, "the window may type again");
}

/// §7 step 3: a restore never writes a fallback draft; a failed save
/// aborts it.
#[test]
fn a_restore_writes_no_fallback_draft_and_aborts_on_a_failed_save() {
    let mut p = timed();
    begin_edit(&mut p);
    begin_stop(&mut p, true);
    p.host.fs.calls.clear();
    p.host.fs.inject(Phase::PageTemp, [Fault::Before]);
    pump(&mut p);
    assert!(first(&p, Phase::PageTemp).is_some());
    assert_eq!(first(&p, Phase::DraftTemp), None, "no fallback draft");
    assert!(p.host.logical_drafts().is_empty());
    assert_eq!(
        state(&p),
        StopState::Aborted(BTreeSet::from(["a.md".into()]))
    );
}

/// §7 step 3: a restore aborts on a conflict and closes only once an
/// earlier draft is retired; it saves the page and removes its draft.
#[test]
fn a_restore_retires_an_earlier_draft_and_aborts_on_a_conflict() {
    let mut p = timed();
    begin_edit(&mut p);
    p.with_host(|h| {
        risk(h, "a.md");
        draft(h, "a.md");
        open(h, "b.md");
        edit(h, "b.md", "mine");
        h.fs.external("b.md", text("theirs"), true);
        assert_eq!(h.observe("b.md"), Disposition::Applied);
    });
    assert!(p.host.logical_drafts().contains_key("a.md"));
    begin_stop(&mut p, true);
    time(&p, 500);
    pump(&mut p);
    assert!(p.host.pages["a.md"].clean());
    assert!(
        !p.host.logical_drafts().contains_key("a.md"),
        "draft retired"
    );
    assert_eq!(
        state(&p),
        StopState::Aborted(BTreeSet::from(["b.md".into()]))
    );
}

/// Q5 (REVIEW-3): a restore over clean pages saves nothing and is ready.
#[test]
fn q5_a_restore_over_clean_pages_manufactures_no_save() {
    let mut p = timed();
    p.with_host(|h| open(h, "a.md"));
    begin_stop(&mut p, true);
    p.host.fs.calls.clear();
    pump(&mut p);
    assert_eq!(first(&p, Phase::PageTemp), None);
    assert_eq!(state(&p), StopState::Ready);
}

/// Put a page at risk with an exact durable draft, as an earlier failed
/// ordinary save leaves it once its draft worker finished.
fn drafted_after_a_failed_save(p: &mut Timed) {
    begin_edit(p);
    p.with_host(|h| {
        risk(h, "a.md");
        draft(h, "a.md");
    });
    assert!(p.host.worker.is_none() && p.host.logical_drafts().contains_key("a.md"));
}

/// V1 (REVIEW-3a): a switch over a page whose exact draft is already
/// durable attempts the page's save before it is ready, even when the
/// caller asks before the driver polls; the draft is the fallback only
/// once that save fails (§6 step 3).
#[test]
fn v1_a_switch_attempts_the_save_of_a_drafted_page_before_it_is_ready() {
    let mut p = timed();
    drafted_after_a_failed_save(&mut p);
    begin_stop(&mut p, false);
    p.host.fs.calls.clear();
    assert_eq!(
        state(&p),
        StopState::Waiting,
        "V1: the switch was ready before its save was attempted"
    );
    pump(&mut p);
    assert!(first(&p, Phase::PageTemp).is_some(), "the save ran");
    assert!(p.host.pages["a.md"].clean());
    assert_eq!(state(&p), StopState::Ready);
}

/// V1 (REVIEW-3a), the reviewer's schedule: a restore over a page with a
/// recoverable draft closes only once the page is saved and the draft
/// retired, so the host relaunched on the restored tree recovers nothing
/// over the backup's bytes (§7 step 3).
#[test]
fn v1_a_restore_never_closes_over_a_recoverable_draft() {
    let mut p = timed();
    drafted_after_a_failed_save(&mut p);
    let backup = p.host.fs.files["graph/a.md"].clone();
    begin_stop(&mut p, true);
    assert_eq!(
        state(&p),
        StopState::Waiting,
        "V1: the restore was ready over a dirty page with a recoverable draft"
    );
    pump(&mut p);
    time(&p, 500);
    pump(&mut p);
    assert!(p.host.pages["a.md"].clean());
    assert!(p.host.logical_drafts().is_empty(), "the draft is retired");
    assert_eq!(state(&p), StopState::Ready);
    assert_eq!(p.with_host(|h| h.switch_finish()), Disposition::Applied);
    // The restore writes the backup's bytes; one fresh host launches.
    p.host.fs.external("a.md", Some(backup.clone()), true);
    p.with_host(|h| {
        h.launch();
        drain(h);
    });
    time(&p, 5000);
    pump(&mut p);
    assert_eq!(
        p.host.fs.files["graph/a.md"], backup,
        "V1: a pre-restore draft was applied over the restored file"
    );
}

/// `page_save_now` (§4.4): a barrier skips only the debounce of a page it
/// needs. A clean page has nothing to hurry, a failed save keeps its
/// backoff, and the urgency ends once the page is clean.
#[test]
fn save_now_skips_the_debounce_but_not_a_failed_saves_backoff() {
    let mut p = timed();
    p.host.test.submits_wait = true;
    p.with_host(|h| open(h, "a.md"));
    p.save_now(&["a.md".into()]);
    p.with_host(|h| edit(h, "a.md", "first"));
    pump(&mut p);
    assert!(
        !p.host.pages["a.md"].clean(),
        "a clean page was not hurried"
    );
    time(&p, 10);
    p.host.fs.inject(Phase::PageTemp, [Fault::Before]);
    p.save_now(&["a.md".into()]);
    pump(&mut p);
    assert!(
        p.host.fs.calls.contains(&Phase::PageTemp),
        "the save ran at once"
    );
    assert!(!p.host.pages["a.md"].clean());
    time(&p, 109);
    pump(&mut p);
    assert!(
        !p.host.pages["a.md"].clean(),
        "the backoff holds after a failure"
    );
    time(&p, 110);
    pump(&mut p);
    assert!(p.host.pages["a.md"].clean());
    assert_eq!(p.host.fs.files["graph/a.md"].as_ref(), &b"first"[..]);
    p.with_host(|h| edit(h, "a.md", "second"));
    pump(&mut p);
    assert!(
        !p.host.pages["a.md"].clean(),
        "the next edit debounces again"
    );
}

/// M4 B1: a key the host no longer holds whose durable draft remains is
/// still a draft candidate, though nothing about it changes again:
/// progress retires its vehicles. A failing unlink keeps the obligation,
/// surfaces the failure and retries until the vehicle is gone.
#[test]
fn m4_b1_an_absent_page_with_a_durable_draft_is_retired() {
    for failing in [false, true] {
        let mut h = host();
        open(&mut h, "a.md");
        edit(&mut h, "a.md", "mine");
        risk(&mut h, "a.md");
        draft(&mut h, "a.md");
        h.with_locks(&BTreeSet::from(["a.md".into()]), |h| {
            h.set_page("a.md", None)
        });
        assert!(h.logical_drafts().contains_key("a.md"));
        let mut p = Progress::new(h, ManualClock(Cell::new(0)));
        if failing {
            p.host.fs.inject(Phase::DraftUnlink, [Fault::Before; 4]);
        }
        let mut now = 0;
        while p.host.logical_drafts().contains_key("a.md") {
            assert!(now < 600_000, "the obsolete draft is never retired");
            pump(&mut p);
            if failing && p.draft_error_pages().contains("a.md") {
                assert!(
                    p.host.logical_drafts().contains_key("a.md"),
                    "a failed unlink keeps its vehicle"
                );
            }
            now += 30_000;
            time(&p, now);
        }
        assert!(p.host.fs.draft_files(true).is_empty(), "failing={failing}");
        assert!(p.draft_error_pages().is_empty(), "failing={failing}");
    }
}

/// M4 R2: a stopped host's drafts are its readable directory. A vehicle
/// renamed but not yet synced is outside the durable index, yet a stop
/// still reads it, and launch recovers it after a process crash; the
/// candidate sets are rebuilt at the stop and at launch (the reference
/// tracker compares every reconcile).
#[test]
fn m4_r2_a_stop_and_launch_rebuild_from_the_readable_directory() {
    let mut p = timed();
    begin_edit(&mut p);
    p.with_host(|h| {
        h.fs.external("a.md", text("theirs"), true);
        h.observe("a.md");
    });
    for _ in 0..3 {
        poll(&mut p);
    } // start, temp, rename
    p.host.fs.inject(Phase::DraftSync, [Fault::Before; 100]);
    poll(&mut p);
    assert!(!p.host.durable.contains_key("a.md"), "not durable yet");
    p.with_host(|h| h.stop());
    assert_eq!(p.host.logical_drafts()["a.md"].bytes, text("first"));
    p.with_host(|h| {
        h.fs.crash();
        assert!(matches!(
            h.launch(),
            Disposition::Applied | Disposition::Pending
        ));
    });
    assert_eq!(p.host.pages["a.md"].buf, text("first"));
    assert!(p.host.pages["a.md"].risk);
    pump(&mut p);
    assert_eq!(p.host.logical_drafts()["a.md"].bytes, text("first"));
}

/// Run the draft worker to its end without moving the clock: an
/// operation's draft, its application and explosion.
fn explode(p: &mut Timed) {
    for _ in 0..200 {
        if p.host.worker.is_none() {
            return;
        }
        poll(p);
    }
    panic!("the draft worker never finished");
}

/// Caller urgency (STEP3-DESIGN M4 notes, REVIEW-progress-plan R1/R5): the
/// pages an operation's caller applied (a rename here) save without the
/// edit debounce once the operation's explosion completes, but a failed
/// save's backoff still applies. The clock stays frozen at the application.
#[test]
fn a_caller_operations_pages_skip_the_debounce_but_not_a_failed_saves_backoff() {
    let mut p = timed();
    time(&p, 1_000);
    let renamed = p.with_host(|h| {
        h.rename(
            "a.md",
            "c.md",
            &BTreeSet::new(),
            "A",
            "C",
            tine_core::config::FileNameFormat::TripleLowbar,
        )
    });
    assert_eq!(renamed, Disposition::Pending);
    explode(&mut p);
    p.host.fs.inject(Phase::PageTemp, [Fault::Before]);
    assert_eq!(poll(&mut p), Disposition::Pending);
    assert_eq!(
        p.host.job.as_ref().map(|job| job.page.as_str()),
        Some("c.md"),
        "the destination saves at once, with no 400 ms debounce"
    );
    for _ in 0..20 {
        if p.host.job.is_none() {
            break;
        }
        poll(&mut p);
    }
    assert!(p.host.job.is_none() && p.notice("c.md").failures == 1);
    poll(&mut p);
    assert!(p.host.job.is_none(), "a failed urgent save still backs off");
    time(&p, 1_100);
    assert_eq!(poll(&mut p), Disposition::Pending);
    assert_eq!(
        p.host.job.as_ref().map(|job| job.page.as_str()),
        Some("c.md")
    );
}

/// A window Move is an ordinary edit and keeps the debounce at a frozen
/// clock; a window Submit does not (PERF-FIXES D1: the window waited).
#[test]
fn a_window_move_keeps_the_debounce_and_a_submit_does_not() {
    let mut p = timed();
    time(&p, 1_000);
    p.with_host(|h| {
        open(h, "a.md");
        open(h, "b.md");
    });
    window_move(&mut p, "source", "receiver");
    assert!(!p.host.pages["a.md"].clean() && !p.host.pages["b.md"].clean());
    for _ in 0..5 {
        poll(&mut p);
        assert!(
            p.host.job.is_none(),
            "a window Move's pages wait for the debounce"
        );
    }
    time(&p, 1_400);
    assert_eq!(poll(&mut p), Disposition::Pending);

    let mut p = timed();
    time(&p, 1_000);
    p.with_host(|h| {
        open(h, "a.md");
        edit(h, "a.md", "typed");
    });
    assert_eq!(poll(&mut p), Disposition::Pending);
    assert!(p.host.job.is_some(), "a window Submit saves at once");
}

#[test]
fn rename_cost_removal_copy_failure_keeps_the_sibling_through_progress() {
    let mut h = super::lazy_operation_group();
    saved(&mut h, "c.md");
    h.fs.external("b.md", text("foreign b"), true);
    assert_eq!(h.observe("b.md"), Disposition::Applied);
    assert_eq!(
        send(&mut h, "a.md", RequestKind::Discard),
        Disposition::Applied
    );
    h.receive("a.md");
    let mut p = Progress::new(h, ManualClock(Cell::new(0)));
    assert_eq!(poll(&mut p), Disposition::Pending);
    assert!(matches!(
        p.host.worker.as_ref().unwrap().application,
        Some(Application::Removal)
    ));
    p.host.fs.inject(Phase::DraftTemp, [Fault::Before]);
    for n in 1..=30 {
        time(&p, n * 30_000);
        poll(&mut p);
        if p.host.worker.is_none() {
            break;
        }
    }
    assert!(p.host.worker.is_none());
    p.with_host(|h| restart(h, true, false));
    assert_eq!(
        p.host.pages.get("b.md").map(|p| p.buf.clone()),
        Some(text("valuable operation b")),
        "RENAME-COST L2: a removal's failed sibling copy retries before the operation file goes"
    );
}

/// REVIEW-rename-cost F1: a page of a rename whose save keeps failing must
/// not keep its published siblings' records in the operation file through a
/// stop. The stop drafts the failing page (its exact record) and explodes the
/// operation file, as before RENAME-COST (`a_switch_drafts_a_page_only_after_its_save_fails`),
/// so a close, switch or exit completes instead of waiting forever.
#[test]
fn rename_cost_a_failing_operation_page_does_not_block_a_stop() {
    let mut h = super::lazy_operation_group();
    saved(&mut h, "c.md");
    let mut p = Progress::new(h, ManualClock(Cell::new(0)));
    p.host.fs.inject(Phase::PageTemp, [Fault::Before; 200]);
    pump(&mut p);
    assert!(p.host.pages["b.md"].risk && !p.host.pages["b.md"].conflict);
    assert!(
        p.host.op_waits("c.md"),
        "precondition: c's record waits for b"
    );
    begin_stop(&mut p, false);
    for n in 1..=20 {
        time(&p, n * 60_000);
        pump(&mut p);
    }
    assert_eq!(state(&p), StopState::Ready);
    let logical = drafts::scan(p.host.fs.draft_files(true)).logical;
    assert!(
        !logical.contains_key("c.md"),
        "the published page's record is gone"
    );
    let (b, record) = (&p.host.pages["b.md"], &logical["b.md"]);
    assert_eq!(
        (&record.bytes, &record.base, record.version),
        (&b.buf, &b.base, b.version),
        "the failing page keeps its exact draft"
    );
}

/// REVIEW-rename-cost R3-1: a stop runs its save-first saves before it
/// explodes an operation vehicle (§6 step 3). With every owed referrer
/// saving, the operation file retires whole: no draft copy is written.
#[test]
fn rename_cost_a_stop_saves_owed_referrers_before_any_draft() {
    let refs: Vec<String> = (0..20).map(|i| format!("r{i:02}.md")).collect();
    let mut fs = ModelFs::default();
    fs.external("a.md", text("A"), true);
    for r in &refs {
        fs.external(r, text("original"), true);
    }
    let keys = ["a.md", "c.md"]
        .into_iter()
        .map(String::from)
        .chain(refs.iter().cloned());
    let locks = keys.map(|k| (k, Arc::new(Mutex::new(())))).collect();
    let mut h = Host::new(fs, locks);
    for r in &refs {
        open(&mut h, r);
    }
    let set: BTreeSet<PageKey> = refs.iter().cloned().collect();
    let renamed = h.rename_with("a.md", "c.md", &set, |bytes, key, _| {
        Ok(if key.starts_with('r') {
            text("operation")
        } else {
            bytes.clone()
        })
    });
    assert_eq!(renamed, Disposition::Pending);
    drain(&mut h);
    saved(&mut h, "c.md");
    assert_eq!(
        h.fs.draft_files(true).len(),
        1,
        "precondition: one operation file"
    );
    let mut p = Progress::new(h, ManualClock(Cell::new(0)));
    begin_stop(&mut p, false);
    p.host.fs.calls.clear();
    for n in 1..=200 {
        if state(&p) != StopState::Waiting {
            break;
        }
        time(&p, n * 30_000);
        poll(&mut p);
    }
    assert_eq!(state(&p), StopState::Ready);
    let copies = p
        .host
        .fs
        .calls
        .iter()
        .filter(|c| **c == Phase::DraftTemp)
        .count();
    assert_eq!(
        copies, 0,
        "R3-1: the stop wrote draft copies its saves made unnecessary"
    );
    assert!(p.host.fs.draft_files(true).is_empty());
}

/// R3-1, availability: draft writes failing (a disk error in app data)
/// while page saves succeed must not abort a stop over a rename whose saves
/// retire the operation file.
#[test]
fn rename_cost_failing_draft_writes_do_not_abort_a_saving_stop() {
    let mut h = super::lazy_operation_group();
    saved(&mut h, "c.md");
    let mut p = Progress::new(h, ManualClock(Cell::new(0)));
    p.host.fs.inject(Phase::DraftTemp, [Fault::Before; 10_000]);
    begin_stop(&mut p, false);
    for n in 1..=200 {
        if state(&p) != StopState::Waiting {
            break;
        }
        time(&p, n * 60_000);
        poll(&mut p);
    }
    assert_eq!(state(&p), StopState::Ready);
}

/// P5 (I-12): draft refresh suppression (`begin_draft_retiring`), the switch
/// barrier (`can_switch`) and Progress ask one question through
/// `Record::holds`: bytes, base and version all equal. Each field differing
/// alone, absent bytes and an UNKNOWN base included, makes it false.
#[test]
fn a_draft_holds_its_page_only_when_bytes_base_and_version_all_match() {
    let bytes = |s: &str| -> Text { Some(Arc::from(s.as_bytes())) };
    let page = |buf: Text, base: Base, version: u64| Page {
        buf,
        base,
        version,
        typed: true,
        risk: true,
        conflict: false,
        obs: None,
    };
    let record = |b: Text, base: Base, version: u64| Record {
        page: "a.md".into(),
        wseq: 1,
        version,
        base,
        bytes: b,
    };
    let known = || Base::Known(bytes("old"));
    for (buf, base) in [
        (bytes("new"), known()),
        (None, known()),
        (bytes("new"), Base::Unknown),
        (None, Base::Unknown),
    ] {
        let p = page(buf.clone(), base.clone(), 7);
        assert!(record(buf.clone(), base.clone(), 7).holds(&p));
        let other = if buf.is_some() { None } else { bytes("new") };
        assert!(!record(other, base.clone(), 7).holds(&p), "bytes");
        let other_base = if base == Base::Unknown {
            known()
        } else {
            Base::Unknown
        };
        assert!(!record(buf.clone(), other_base, 7).holds(&p), "base");
        assert!(
            !record(buf.clone(), Base::Known(bytes("x")), 7).holds(&p),
            "base bytes"
        );
        assert!(!record(buf, base, 8).holds(&p), "version");
    }
}
