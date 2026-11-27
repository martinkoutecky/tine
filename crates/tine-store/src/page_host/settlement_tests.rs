//! Durable rename recovery, slice 2 review round 2 (rcd-impl-2e): the
//! superseding decision is durable before newer source text publishes, in
//! the uncertain-write and the rewrite windows (F2), and a stalled
//! operation rewrites its settled pairs out at once (F1); two durable
//! superseding decisions on one source are acted on once (F9). Fixture: an
//! operation of `k` disjoint moves `s<i>.md -> t<i>.md` on the model
//! filesystem, admitted through `Host::apply`.
use super::*;
use crate::page_host::order::{PairPhase, Removal};

fn src(i: usize) -> String {
    format!("s{i}.md")
}

fn dst(i: usize) -> String {
    format!("t{i}.md")
}

/// The admitted operation of `k` moves, its paired draft durable; nothing
/// published yet.
fn admitted(k: usize) -> Host<ModelFs> {
    let mut fs = ModelFs::default();
    for i in 0..k {
        fs.external(&src(i), text(&format!("S{i}")), true);
    }
    let locks = (0..k)
        .flat_map(|i| [src(i), dst(i)])
        .map(|key| (key, Arc::new(Mutex::new(()))))
        .collect();
    let mut h = Host::new(fs, locks);
    let effects = (0..k)
        .flat_map(|i| {
            let bytes = text(&format!("S{i}"));
            [(src(i), (None, bytes.clone())), (dst(i), (bytes, None))]
        })
        .collect();
    let moves: Vec<_> = (0..k).map(|i| (dst(i), src(i))).collect();
    assert_eq!(h.apply(&effects, &moves), Disposition::Pending);
    drain(&mut h);
    operation(&h);
    h
}

/// The one paired operation's vehicle name.
fn operation(h: &Host<ModelFs>) -> String {
    let mut names = (h.live_ops.keys()).filter(|name| h.order.paired(name).is_some());
    let name = names.next().expect("a paired operation").clone();
    assert!(names.next().is_none(), "one paired operation");
    name
}

/// Durable superseding decisions on `key` (the source of a pair).
fn durable_superseding(h: &Host<ModelFs>, key: &str) -> usize {
    let scan = drafts::scan(h.fs.draft_files(true));
    // `p-<page hash>-`: every vehicle name for `key` starts with it.
    let prefix = &drafts::page_name(key)[..35];
    (scan.decisions.iter())
        .filter(|(name, d)| name.starts_with(prefix) && d.choice.superseding())
        .count()
}

/// Drive `key`'s started save to its terminal.
fn finish_save(h: &mut Host<ModelFs>, key: &str) {
    for _ in 0..10 {
        if h.job.is_none() {
            break;
        }
        let epoch = h.fs.epochs.get(key).copied().unwrap_or(0);
        h.advance_save_at(epoch);
    }
    assert!(h.pages[key].clean(), "{key} saved");
}

/// F2, uncertain write: the superseding decision's vehicle neither reached
/// Present nor is durably absent. The save waits; its retry writes the
/// decision again, so it is durable before the newer text publishes, and a
/// crash right after that publication relaunches with the newer text, not
/// the reinstalled removal. Before, the uncertain write marked the pair
/// Superseded and the retry published with no durable decision behind it.
#[test]
fn an_uncertain_superseding_write_is_retried_before_the_text_publishes() {
    let mut h = admitted(2);
    saved(&mut h, &dst(0));
    saved(&mut h, &dst(1));
    edit(&mut h, &src(0), "S0 newer");
    // Sync fails until the vehicle's cleanup sync is left owed.
    h.fs.inject(Phase::DraftSync, [Fault::Before; 8]);
    assert_eq!(h.start_save(&src(0)), Disposition::Disabled);
    assert!(h.job.is_none());
    h.fs.faults.clear();
    assert!(
        h.order.supersedes(&src(0)).is_some(),
        "an uncertain decision is not a durable one"
    );
    assert_eq!(h.start_save(&src(0)), Disposition::Pending);
    assert_eq!(
        durable_superseding(&h, &src(0)),
        1,
        "the decision is durable before the newer text can publish"
    );
    finish_save(&mut h, &src(0));
    restart(&mut h, false, false);
    relaunched_with_newer_text(&mut h);
}

/// After a relaunch the newer text on `s0` stands: launch adopted the
/// superseding decision, so no pair arms its removal again (the pair keeps
/// custody of the original text until its receiver is witnessed).
fn relaunched_with_newer_text(h: &mut Host<ModelFs>) {
    assert!(
        !matches!(h.order.removal(&src(0)), Removal::Checked { .. }),
        "no removal replays over the newer text"
    );
    open(h, &src(0));
    assert_eq!(h.pages[&src(0)].buf, text("S0 newer"));
    assert_eq!(
        h.fs.files[&format!("graph/{}", src(0))].as_ref(),
        b"S0 newer"
    );
}

/// F2, rewrite window: while a settlement rewrite's unlink of the original
/// draft keeps failing, the operation's pages belong to the rewrite worker,
/// so newer text on a surviving source waits and writes no decision under
/// either request id. Once the rewrite commits, its decision is written
/// under the new draft's id and launch acts on it (it is not the stale
/// decision of an uncommitted copy).
#[test]
fn newer_source_text_waits_out_a_failing_rewrite_unlink() {
    let mut h = admitted(2);
    saved(&mut h, &dst(0));
    saved(&mut h, &dst(1));
    saved(&mut h, &src(1));
    edit(&mut h, &src(0), "S0 newer");
    let old = operation(&h);
    assert!(h.live_ops[&old].owed.is_empty(), "nothing owed: it stalls");
    assert_eq!(h.begin_draft_retiring(&src(1)), Disposition::Pending);
    assert!(matches!(
        h.worker.as_ref().unwrap().application,
        Some(Application::Rewrite(_))
    ));
    h.fs.inject(Phase::DraftUnlink, [Fault::Before; 6]);
    for _ in 0..40 {
        let w = h.worker.as_ref().unwrap();
        if w.task.name == old && w.task.failures > 0 {
            break;
        }
        h.advance_draft();
    }
    let w = h.worker.as_ref().unwrap();
    assert!(w.task.name == old && w.task.failures > 0, "unlink failing");
    let files = h.fs.draft_files(false).len();
    assert_eq!(h.start_save(&src(0)), Disposition::Waiting);
    assert_eq!(h.fs.draft_files(false).len(), files, "no decision written");
    h.fs.faults.clear();
    drain(&mut h);
    let new = operation(&h);
    assert_ne!(new, old, "the rewrite committed");
    assert_eq!(h.start_save(&src(0)), Disposition::Pending);
    assert_eq!(durable_superseding(&h, &src(0)), 1);
    finish_save(&mut h, &src(0));
    let scan = drafts::scan(h.fs.draft_files(true));
    assert!(scan.stale_decisions.is_empty() && scan.rewrites.is_empty());
    restart(&mut h, false, false);
    relaunched_with_newer_text(&mut h);
}

/// F9 (rcd-impl-2 round 2): newer text on `s0` whose first superseding
/// write ends uncertain with its cleanup unlinks failing, so the retry
/// writes a second decision under a fresh name and both are durable.
fn doubled_superseding(h: &mut Host<ModelFs>) {
    edit(h, &src(0), "S0 newer");
    h.fs.inject(Phase::DraftSync, [Fault::Before; 3]);
    h.fs.inject(Phase::DraftUnlink, [Fault::Before; 8]);
    assert_eq!(h.start_save(&src(0)), Disposition::Disabled);
    h.fs.faults.clear();
    assert!(h.order.supersedes(&src(0)).is_some(), "still owed");
    assert_eq!(h.start_save(&src(0)), Disposition::Pending);
    assert_eq!(
        durable_superseding(h, &src(0)),
        2,
        "two durable superseding decisions on one source"
    );
    // Scan acts on both: neither blocks the operation nor is stale.
    let scan = drafts::scan(h.fs.draft_files(true));
    assert!(scan.blocking.is_empty(), "{:?}", scan.blocking);
    assert!(scan.stale_decisions.is_empty() && scan.rewrites.is_empty());
    finish_save(h, &src(0));
}

/// Run the progress loop's retirement steps over every page of the
/// operation until none starts; how many of them were rewrites.
fn retire_all(h: &mut Host<ModelFs>, k: usize) -> usize {
    let mut rewrites = 0;
    let keys: Vec<String> = (0..k).flat_map(|i| [src(i), dst(i)]).collect();
    for _ in 0..4 * k {
        if !keys
            .iter()
            .any(|key| h.begin_draft_retiring(key) == Disposition::Pending)
        {
            return rewrites;
        }
        let rewrite = matches!(
            h.worker.as_ref().unwrap().application,
            Some(Application::Rewrite(_))
        );
        rewrites += usize::from(rewrite);
        drain(h);
    }
    panic!("retirement keeps starting");
}

/// F9, retirement: with both decisions durable and the receiver witnessed,
/// the newer text disarms the pair once; when the sibling settles the whole
/// operation retires with both decisions, leaving no draft, and a relaunch
/// keeps the newer text.
#[test]
fn two_superseding_decisions_on_one_source_retire_with_the_operation() {
    let mut h = admitted(2);
    saved(&mut h, &dst(0));
    saved(&mut h, &dst(1));
    doubled_superseding(&mut h);
    let name = operation(&h);
    let phase = |h: &Host<ModelFs>, i: usize| {
        let paired = h.order.paired(&name).unwrap();
        paired
            .pairs
            .iter()
            .find(|p| p.source == src(i))
            .unwrap()
            .phase
    };
    assert_eq!(phase(&h, 0), PairPhase::Disarmed);
    saved(&mut h, &src(1));
    assert_eq!(phase(&h, 1), PairPhase::Settled);
    assert_eq!(retire_all(&mut h, 2), 0, "whole retirement, no rewrite");
    assert!(
        h.fs.draft_files(false).is_empty(),
        "every record and decision retired: {:?}",
        h.fs.draft_files(false)
            .iter()
            .map(|(n, _)| n)
            .collect::<Vec<_>>()
    );
    restart(&mut h, false, false);
    relaunched_with_newer_text(&mut h);
}

/// F9, launch: a crash with both decisions durable and the sibling still
/// armed. Launch adopts both (the newer text stands, no removal replays
/// over it); having no receiver witness it keeps the original's custody
/// (Retained), while the sibling's removal still recovers and settles.
#[test]
fn two_superseding_decisions_on_one_source_relaunch() {
    let mut h = admitted(2);
    saved(&mut h, &dst(0));
    saved(&mut h, &dst(1));
    doubled_superseding(&mut h);
    restart(&mut h, false, false);
    relaunched_with_newer_text(&mut h);
    // Either may go once the other stays durable; at least one stays.
    assert!(
        durable_superseding(&h, &src(0)) >= 1,
        "launch keeps a decision"
    );
    assert!(
        matches!(h.order.removal(&src(1)), Removal::Checked { .. }),
        "the sibling pair's removal still recovers"
    );
    // The recovered receiver publishes again (its witness), then the source.
    saved(&mut h, &dst(1));
    saved(&mut h, &src(1));
    let paired = h.order.paired(&operation(&h)).unwrap();
    let phases: Vec<_> = (0..2)
        .map(|i| {
            paired
                .pairs
                .iter()
                .find(|p| p.source == src(i))
                .unwrap()
                .phase
        })
        .collect();
    assert_eq!(phases, [PairPhase::Retained, PairPhase::Settled]);
    let logical = drafts::scan(h.fs.draft_files(true)).logical;
    assert_eq!(logical[&src(0)].base, Base::Known(text("S0")));
}

/// F9, settlement rewrite: the same two decisions on a Retained pair (its
/// receiver carries no witness) survive the stalled operation's rewrite of
/// its settled sibling, each copied under the new draft's id, and the next
/// launch still keeps the newer text and the original's custody.
#[test]
fn two_superseding_decisions_survive_a_settlement_rewrite() {
    let mut h = admitted(2);
    saved(&mut h, &dst(0));
    saved(&mut h, &dst(1));
    // Another program writes the same bytes: no witness of this host.
    h.fs.external(&dst(0), text("S0"), true);
    doubled_superseding(&mut h);
    saved(&mut h, &src(1));
    let old = operation(&h);
    assert!(h.live_ops[&old].owed.is_empty() && h.order.holds(&old));
    assert_eq!(retire_all(&mut h, 2), 1, "one rewrite drops the sibling");
    let new = operation(&h);
    assert_ne!(new, old, "the rewrite committed");
    assert!(
        durable_superseding(&h, &src(0)) >= 1,
        "the rewrite carries a durable superseding decision over"
    );
    let scan = drafts::scan(h.fs.draft_files(true));
    assert!(scan.blocking.is_empty() && scan.stale_decisions.is_empty());
    restart(&mut h, false, false);
    relaunched_with_newer_text(&mut h);
    let paired = h.order.paired(&operation(&h)).unwrap();
    assert_eq!(paired.pairs.len(), 1);
    assert_eq!(paired.pairs[0].phase, PairPhase::Retained);
    let logical = drafts::scan(h.fs.draft_files(true)).logical;
    assert_eq!(logical[&src(0)].base, Base::Known(text("S0")));
}

/// What a stalled operation's settlement costs: rewrites, draft bytes
/// written, draft-directory syncs.
#[derive(Debug, PartialEq, Eq)]
struct Cost {
    rewrites: usize,
    bytes: usize,
    syncs: usize,
}

/// An operation of `k` moves stalls: pair 0 is Retained (newer source text
/// published while its receiver carries no witness), every other pair is
/// settled and nothing is owed. Run the progress loop's retirement steps
/// until none starts, counting what the settlement rewrites cost.
fn stalled_settlement(k: usize) -> Cost {
    let mut h = admitted(k);
    for i in 0..k {
        saved(&mut h, &dst(i));
    }
    // Another program writes the same bytes: no witness of this host.
    h.fs.external(&dst(0), text("S0"), true);
    edit(&mut h, &src(0), "S0 newer");
    saved(&mut h, &src(0));
    for i in 1..k {
        saved(&mut h, &src(i));
    }
    let name = operation(&h);
    assert!(h.live_ops[&name].owed.is_empty());
    assert!(h.order.holds(&name), "the retained pair stalls it");
    let syncs = |h: &Host<ModelFs>| {
        h.fs.calls
            .iter()
            .filter(|p| **p == Phase::DraftSync)
            .count()
    };
    let before = syncs(&h);
    let mut cost = Cost {
        rewrites: 0,
        bytes: 0,
        syncs: 0,
    };
    loop {
        let started = (0..k).find(|&i| h.begin_draft_retiring(&src(i)) == Disposition::Pending);
        if started.is_none() {
            break;
        }
        if matches!(
            h.worker.as_ref().unwrap().application,
            Some(Application::Rewrite(_))
        ) {
            cost.rewrites += 1;
        }
        while let Some(w) = &h.worker {
            if w.task.stage == Stage::Temp {
                cost.bytes += w.task.bytes.as_ref().map_or(0, Vec::len);
            }
            h.advance_draft();
        }
    }
    cost.syncs = syncs(&h) - before;
    // The retained pair keeps the only copy of S0 in the operation draft.
    let paired = h.order.paired(&operation(&h)).unwrap();
    assert_eq!(paired.pairs.len(), 1);
    assert_eq!(paired.pairs[0].phase, PairPhase::Retained);
    let logical = drafts::scan(h.fs.draft_files(true)).logical;
    assert_eq!(logical[&src(0)].base, Base::Known(text("S0")));
    cost
}

/// F1 (PC-1): a stalled operation with K−1 settled pairs rewrites once,
/// dropping all of them, not once per settled pair (O(K) rewrites of O(K)
/// bytes each before). Measured: K = 8 and K = 64.
#[test]
fn a_stalled_operation_rewrites_its_settled_pairs_out_at_once() {
    for k in [8, 64] {
        let cost = stalled_settlement(k);
        eprintln!("stalled settlement K={k}: {cost:?}");
        assert_eq!(cost.rewrites, 1, "K={k}: one rewrite, {cost:?}");
    }
}
