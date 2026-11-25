//! Quint s3.2: a rename's in-run write order (gates), its ghost obligations
//! and the two invariants checked beside the guarantee.
use super::*;

/// (page, dep, ver, dst): page's save waits until dep publishes ver.
pub(super) type Obligations = BTreeSet<(usize, usize, i64, usize)>;

/// Quint: gates1
fn gates1(x: &State, gt: &Gate, k: usize) -> bool {
    let receiver = |p| gt.moves.iter().any(|&(dst, _)| dst == p);
    let source = |p| gt.moves.iter().any(|&(_, src)| src == p);
    gt.open.contains(&k)
        && !receiver(k)
        && !(x.config.mutant("MRF") && !source(k))
        && (gt.open.iter().copied().any(receiver)
            || (source(k) && gt.open.iter().any(|&p| !source(p)) && !x.config.mutant("MRD")))
}

/// Quint: gateObligations
pub(super) fn gate_obligations(gt: &Gate) -> Obligations {
    let receiver = |p| gt.moves.iter().any(|&(dst, _)| dst == p);
    let source = |p| gt.moves.iter().any(|&(_, src)| src == p);
    let mut obligations = BTreeSet::new();
    for &(dst, src) in &gt.moves {
        for &p in gt.open.iter().filter(|&&p| !receiver(p)) {
            if gt.open.contains(&dst) {
                obligations.insert((p, dst, gt.vers[dst], dst));
            }
            if gt.open.contains(&src) && !source(p) {
                obligations.insert((src, p, gt.vers[p], dst));
            }
        }
    }
    obligations
}
/// Quint: gated
pub(super) fn gated(x: &State, k: usize) -> bool {
    x.s.gates.iter().any(|gt| gates1(x, gt, k))
}
/// Quint: touches
pub(super) fn touches(x: &State, ks: &BTreeSet<usize>) -> bool {
    x.s.gates
        .iter()
        .any(|gt| ks.iter().any(|&k| gt.vers[k] != NONE))
        || (!x.config.mutant("MForgetTouches") && ks.iter().any(|&k| retained(&x.s, k)))
}
/// Quint: partner (None for NONE)
pub(super) fn partner(x: &State, k: usize) -> Option<usize> {
    let s = &x.s;
    s.pairs
        .iter()
        .rfind(|pair| {
            pair.dst == k
                && pair.unresolved()
                && matches!(pair.decision, Decision::None | Decision::Unreadable)
                && (awaits(s, k)
                    || !pair.certified(&s.disk, &s.stable, x.config.weak)
                    || matches!(pair.phase, Phase::Held | Phase::Moved | Phase::Blocked))
        })
        .map(|pair| pair.src)
        .or_else(|| {
            s.gates
                .iter()
                .filter(|gt| gt.open.contains(&k))
                .flat_map(|gt| &gt.moves)
                .rfind(|&&(dst, _)| dst == k)
                .map(|&(_, src)| src)
        })
}
/// Quint: discharge
pub(super) fn discharge(s: &mut Sys, q: usize, ver: i64) {
    for gt in &mut s.gates {
        if gt.vers[q] != NONE && ver >= gt.vers[q] {
            gt.open.remove(&q);
        }
    }
    s.gates.retain(|gt| !gt.open.is_empty());
}
/// s3.2, in upDiscard: a Discard of a running rename's unwitnessed
/// destination reverts its source while that still holds the deletion.
pub(super) fn revert_partner(
    x: &State,
    s: &mut Sys,
    g: &mut Ghost,
    source: Option<usize>,
    v1: i64,
) {
    let Some(src) =
        source.filter(|&q| (s.pages[q].held || x.config.mutant("MRH")) && s.pages[q].buf == ABSENT)
    else {
        return;
    };
    let sp = s.pages[src].clone();
    let sd = x.s.disk[src];
    let v2 = next(v1);
    let spr = &g.promise[src];
    let shold = sp.risk && (sd == sp.buf || sd == x.s.drafts[src].bytes) && !x.config.mutant("MDE");
    let slive = spr.on && !(spr.saved && g.ext[src] > spr.ep);
    if !(slive && spr.bytes == sd) {
        g.promise[src] = end_promise(spr, v2);
    }
    s.pages[src] = Page {
        buf: sd,
        base: sd,
        ver: v2,
        obs: sd,
        conflict: false,
        risk: shold,
        typed: false,
        ..sp
    };
    g.vc = v2;
    g.mine.insert(src);
}
/// Quint: discarded
pub(super) fn discarded(s: &mut Sys, k: usize, source: Option<usize>) {
    for gt in &mut s.gates {
        if gt.open.remove(&k) || source.is_some() {
            gt.moves.retain(|&(dst, src)| {
                if dst == k {
                    gt.open.remove(&src);
                }
                dst != k
            });
        }
    }
    s.gates
        .retain(|gt| !gt.moves.is_empty() && !gt.open.is_empty());
}
/// Quint: oblPublished
pub(super) fn obl_published(obl: &mut Obligations, q: usize, ver: i64) {
    obl.retain(|o| !(o.1 == q && ver >= o.2));
}
/// Quint: oblDiscarded
pub(super) fn obl_discarded(obl: &mut Obligations, k: usize, source: Option<usize>) {
    obl.retain(|o| !(o.0 == k || o.1 == k || source.is_some_and(|src| o.0 == src || o.3 == k)));
}
/// Quint: unrename
pub(super) fn unrename(renamed: &mut BTreeSet<(usize, usize)>, ks: &[usize]) {
    renamed.retain(|r| !(ks.contains(&r.0) || ks.contains(&r.1)));
}
/// The save-start ghost check (flush, flushDel).
pub(super) fn started(g: &mut Ghost, p: usize) {
    g.order_ok = g.order_ok && !g.obl.iter().any(|o| o.0 == p);
}
/// Quint: orderHolds
pub(super) fn order_holds(x: &State) -> bool {
    x.g.order_ok && x.s.owners.len() <= 1
}
/// Quint: renameResolves
pub(super) fn rename_resolves(x: &State) -> bool {
    x.g.renamed
        .iter()
        .all(|r| x.s.disk[r.0] != ABSENT || x.s.disk[r.1] != ABSENT)
        && x.s.pairs.iter().all(|pair| {
            let preimage = pair.records[pair.src].base;
            let image = pair.records[pair.dst].bytes;
            x.s.disk[pair.src] == preimage
                || x.s.disk[pair.dst] == image
                || (preimage != ABSENT && preimage != NONE)
        })
}
