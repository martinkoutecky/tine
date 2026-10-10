//! Quint s3 page operations; pure rules, no storage wiring.
use super::order::touches;
use super::*;

/// Quint: opCur
pub(super) fn op_cur(s: &Sys, p: usize) -> Text {
    if s.pages[p].held {
        s.pages[p].buf
    } else {
        s.disk[p]
    }
}
/// Quint: opEdit
pub(super) fn op_edit(s: &Sys, p: usize, t: Text, v: i64) -> Page {
    let base = if s.pages[p].held {
        s.pages[p].clone()
    } else {
        Page {
            held: true,
            buf: s.disk[p],
            base: s.disk[p],
            obs: s.disk[p],
            ..nopage()
        }
    };
    Page {
        buf: t,
        typed: true,
        ver: v,
        ..base
    }
}
/// Quint: opClean
pub(super) fn op_clean(s: &Sys, p: usize) -> bool {
    !s.pages[p].held || clean(&s.pages[p])
}
/// Quint s3.1: load — one path, no window request or draft effect.
pub(super) fn load(x: &State, p: usize) -> Option<State> {
    if !x.s.alive || x.s.pages[p].held {
        return None;
    }
    let mut s = x.s.clone();
    let mut g = x.g.clone();
    g.vc = next(g.vc);
    s.pages[p] = Page {
        held: true,
        buf: s.disk[p],
        base: s.disk[p],
        obs: s.disk[p],
        ver: g.vc,
        ..nopage()
    };
    commit(x, s, g, "load")
}
/// Quint s3.1: opHeld; MUL deliberately restores read-inside-operation.
fn op_held(x: &State, ps: &BTreeSet<usize>) -> bool {
    x.config.mutant("MUL") || ps.iter().all(|p| x.s.pages[*p].held)
}
/// Quint: opFree
pub(super) fn op_free(x: &State, ps: &BTreeSet<usize>) -> bool {
    !(x.s.job.on && ps.contains(&x.s.job.p))
}
/// Quint: opRename
pub(super) fn op_rename(
    x: &State,
    src: usize,
    dst: usize,
    refs: &BTreeSet<usize>,
    rt: &std::collections::BTreeMap<usize, Text>,
) -> Option<State> {
    let text = op_cur(&x.s, src);
    let full = text != ABSENT;
    let named = if full {
        BTreeSet::from([src, dst])
    } else {
        BTreeSet::new()
    };
    let all3: BTreeSet<_> = named.union(refs).copied().collect();
    if !x.s.alive
        || src == dst
        || !op_free(x, &all3)
        || !op_held(x, &all3.union(&BTreeSet::from([src])).copied().collect())
        || !refs.iter().all(|r| *r != src && *r != dst)
        || touches(
            &x.s,
            &refs.union(&BTreeSet::from([src, dst])).copied().collect(),
        )
        || (full && !op_clean(&x.s, src))
        || (full
            && !((op_cur(&x.s, dst) == ABSENT && op_clean(&x.s, dst)) || x.config.mutant("MRN3")))
        || !refs
            .iter()
            .all(|r| op_clean(&x.s, *r) || x.config.mutant("MRN2"))
        || all3.is_empty()
    {
        return None;
    }
    let versions: Vec<_> = (0..x.s.pages.len())
        .scan(x.g.vc, |v, _| {
            *v = next(*v);
            Some(*v)
        })
        .collect();
    let mut s = x.s.clone();
    let mut g = x.g.clone();
    if full {
        let mut nd = op_edit(&x.s, dst, text, versions[dst]);
        nd.risk = true;
        let mut ns = op_edit(&x.s, src, ABSENT, versions[src]);
        ns.risk = true;
        if !x.config.mutant("MRN1") {
            s.drafts[dst] = draft_entry(&nd);
        }
        if !x.config.mutant("MRN5") {
            s.drafts[src] = draft_entry(&ns);
        }
        s.pages[dst] = nd;
        s.pages[src] = ns;
        // s3.2: the order over the pages this rename changes, and the
        // obligations the ghost holds it to.
        s.gates.push(Gate {
            dst,
            src,
            vers: (0..versions.len())
                .map(|p| if all3.contains(&p) { versions[p] } else { NONE })
                .collect(),
            open: all3.clone(),
        });
        for &r in refs {
            g.obl.insert((r, dst, versions[dst], dst));
            g.obl.insert((src, r, versions[r], dst));
        }
        g.obl.insert((src, dst, versions[dst], dst));
    }
    unrename(&mut g.renamed, &[src, dst]);
    if full && x.s.disk[src] != ABSENT {
        g.renamed.insert((src, dst));
    }
    for &r in refs {
        let mut nr = op_edit(&x.s, r, rt.get(&r).copied().unwrap_or(ABSENT), versions[r]);
        nr.risk = true;
        if !x.config.mutant("MRN4") {
            s.drafts[r] = draft_entry(&nr);
        }
        s.pages[r] = nr;
    }
    let mut gotten: Pairs = refs.iter().map(|r| (*r, rt[r])).collect();
    if full {
        gotten.insert((dst, text));
        if !x.config.mutant("MRN5") {
            gotten.insert((src, ABSENT));
        }
    }
    for p in 0..x.s.pages.len() {
        if p == dst && full {
            if op_cur(&x.s, dst) == ABSENT {
                g.op_read[p].insert(ABSENT);
            }
        } else if (p == src && full) || refs.contains(&p) {
            g.op_read[p].insert(if x.s.pages[p].held {
                x.s.pages[p].base
            } else {
                x.s.disk[p]
            });
        }
    }
    g.vc = *versions.last().unwrap();
    g.mine = named
        .union(
            &refs
                .iter()
                .copied()
                .filter(|r| op_clean(&x.s, *r))
                .collect(),
        )
        .copied()
        .collect();
    for (p, t) in gotten {
        g.wrote[p].insert((t, versions[p]));
        g.promise[p] = ack(&g.promise[p], t, versions[p], false, 0);
    }
    if !all3.iter().all(|p| op_clean(&x.s, *p)) {
        g.bad.insert("D4-unclean-page-changed".into());
    }
    if full && op_cur(&x.s, dst) != ABSENT {
        g.bad.insert("rename-clobbered-target".into());
    }
    commit(x, s, g, if full { "opRename" } else { "opRenameRefs" })
}
/// Quint: opDelete
pub(super) fn op_delete(x: &State, p: usize) -> Option<State> {
    let pg = &x.s.pages[p];
    if !x.s.alive
        || !op_free(x, &BTreeSet::from([p]))
        || !op_held(x, &BTreeSet::from([p]))
        || !op_clean(&x.s, p)
        || op_cur(&x.s, p) == ABSENT
        || touches(&x.s, &BTreeSet::from([p]))
    {
        return None;
    }
    let v1 = next(x.g.vc);
    let mut np = op_edit(&x.s, p, ABSENT, v1);
    np.risk = !x.config.mutant("MDD");
    let mut s = x.s.clone();
    let mut g = x.g.clone();
    if !x.config.mutant("MDD") {
        s.drafts[p] = draft_entry(&np);
        g.wrote[p].insert((ABSENT, v1));
        g.promise[p] = ack(&g.promise[p], ABSENT, v1, false, 0);
    }
    s.pages[p] = np;
    g.vc = v1;
    g.mine = BTreeSet::from([p]);
    unrename(&mut g.renamed, &[p]);
    g.op_read[p].insert(if pg.held { pg.base } else { x.s.disk[p] });
    commit(x, s, g, "opDelete")
}
/// Quint: flushDel
pub(super) fn flush_del(x: &State, p: usize) -> Option<State> {
    let pg = &x.s.pages[p];
    if !x.s.alive
        || x.s.job.on
        || !pg.held
        || pg.conflict
        || pg.buf != ABSENT
        || !dirty(pg)
        || (!x.config.mutant("MDF") && gated(x, p))
    {
        return None;
    }
    let mut s = x.s.clone();
    let mut g = x.g.clone();
    started(&mut g, p);
    s.job = Job {
        on: true,
        p,
        bytes: pg.buf,
        base: pg.base,
        ver: pg.ver,
        phase: 1,
        ep: 0,
    };
    commit(x, s, g, "flush")
}
