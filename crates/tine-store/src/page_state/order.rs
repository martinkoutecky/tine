//! Quint s3.2: a rename's in-run write order (gates), its ghost obligations
//! and the two invariants checked beside the guarantee.
use super::*;

/// (page, dep, ver, dst): page's save waits until dep publishes ver.
pub(super) type Obligations = BTreeSet<(usize, usize, i64, usize)>;

/// Quint: gates1
fn gates1(x: &State, gt: &Gate, k: usize) -> bool {
    gt.open.contains(&k)
        && k != gt.dst
        && !(x.config.mutant("MRF") && k != gt.src)
        && (gt.open.contains(&gt.dst)
            || (k == gt.src && gt.open.len() > 1 && !x.config.mutant("MRD")))
}
/// Quint: gated
pub(super) fn gated(x: &State, k: usize) -> bool {
    x.s.gates.iter().any(|gt| gates1(x, gt, k))
}
/// Quint: touches
pub(super) fn touches(s: &Sys, ks: &BTreeSet<usize>) -> bool {
    s.gates
        .iter()
        .any(|gt| ks.iter().any(|&k| gt.vers[k] != NONE))
}
/// Quint: partner (None for NONE)
pub(super) fn partner(s: &Sys, k: usize) -> Option<usize> {
    s.gates
        .iter()
        .find(|gt| gt.dst == k && gt.open.contains(&k))
        .map(|gt| gt.src)
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
pub(super) fn revert_partner(x: &State, s: &mut Sys, g: &mut Ghost, p: usize, v1: i64) {
    let Some(src) = partner(s, p).filter(|&q| s.pages[q].buf == ABSENT) else {
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
pub(super) fn discarded(s: &mut Sys, k: usize) {
    s.gates.retain(|gt| !(gt.dst == k && gt.open.contains(&k)));
    for gt in &mut s.gates {
        if gt.dst != k {
            gt.open.remove(&k);
        }
    }
    s.gates.retain(|gt| !gt.open.is_empty());
}
/// Quint: oblPublished
pub(super) fn obl_published(obl: &mut Obligations, q: usize, ver: i64) {
    obl.retain(|o| !(o.1 == q && ver >= o.2));
}
/// Quint: oblDiscarded
pub(super) fn obl_discarded(obl: &mut Obligations, k: usize) {
    let cancels = obl.iter().any(|o| o.1 == k && o.3 == k);
    obl.retain(|o| !(o.0 == k || o.1 == k || (cancels && o.3 == k)));
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
    x.g.order_ok
}
/// Quint: renameResolves
pub(super) fn rename_resolves(x: &State) -> bool {
    x.g.renamed
        .iter()
        .all(|r| x.s.disk[r.0] != ABSENT || x.s.disk[r.1] != ABSENT)
}
