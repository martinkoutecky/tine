//! Save and draft transitions of the Quint oracle.
use super::*;

/// Quint: observe
pub(super) fn observe(x: &State, p: usize) -> Option<State> {
    let pg = &x.s.pages[p];
    let d = x.s.disk[p];
    let v1 = next(x.g.vc);
    let nw = table(x, pg, d, v1);
    if !x.s.alive || !pg.held || locked(x, p) {
        return None;
    }
    let mut s = x.s.clone();
    resume_pair(&x.config, &mut s, p);
    if nw == *pg && s.pairs == x.s.pairs {
        return None;
    }
    let mut g = x.g.clone();
    if nw.ver == v1 {
        g.vc = v1;
    }
    s.pages[p] = nw;
    commit(x, s, g, "observe")
}
/// Quint: flush
pub(super) fn flush(x: &State, p: usize) -> Option<State> {
    let pg = &x.s.pages[p];
    if !x.s.alive
        || x.s.job.on
        || !pg.held
        || pg.conflict
        || pg.buf == ABSENT
        || !dirty(pg)
        || gated(x, p)
    {
        return None;
    }
    let mut s = x.s.clone();
    let mut g = x.g.clone();
    s.job = Job {
        on: true,
        p,
        bytes: pg.buf,
        base: pg.base,
        ver: pg.ver,
        phase: 1,
        ep: 0,
    };
    started(&mut g, p);
    commit(x, s, g, "flush")
}
/// Quint: check
pub(super) fn check(x: &State) -> Option<State> {
    let j = &x.s.job;
    if !x.s.alive || !j.on || j.phase != 1 {
        return None;
    }
    let p = j.p;
    let d = x.s.disk[p];
    let mut s = x.s.clone();
    let mut g = x.g.clone();
    let exact = d == j.bytes && awaits(&s, p) && !x.config.mutant("MNoResave");
    let pair = s
        .pairs
        .iter()
        .rfind(|pair| j.bytes == ABSENT && pair.source(p));
    let diverged = pair.filter(|pair| !pair.certified(&s.disk, &s.stable, x.config.weak));
    let v1 = next(g.vc);
    let name = if d != j.base && !exact && !x.config.mutant("MC") && !x.config.mutant("MC1") {
        let nw = table(x, &s.pages[p], d, v1);
        if nw.ver == v1 {
            g.vc = v1;
        }
        s.pages[p] = nw;
        s.job = nojob();
        resume_pair(&x.config, &mut s, p);
        if x.config.mutant("MCV") {
            discharge(&mut s, p, j.ver);
        }
        "mismatch"
    } else if let Some(pair) = diverged.filter(|_| !x.config.mutant("MSkipPrecheck")) {
        let dst = pair.dst;
        let phase = if pair.restorable(&s.disk, &s.trash) {
            Phase::Moved
        } else {
            Phase::Held
        };
        if s.pages[dst].held {
            s.pages[dst] = table(x, &s.pages[dst], s.disk[dst], v1);
            if s.pages[dst].ver == v1 {
                g.vc = v1;
            }
        }
        for pair in s.pairs.iter_mut().filter(|pair| pair.source(p)) {
            pair.phase = phase;
        }
        s.job = nojob();
        "pairHeld"
    } else {
        if exact {
            s.job.base = d;
        }
        s.job.phase = 2;
        g.guard = d == j.base || exact;
        g.order_ok &= diverged.is_none();
        "check"
    };
    commit(x, s, g, name)
}
/// Quint: rename
pub(super) fn rename(x: &State) -> Option<State> {
    let j = &x.s.job;
    if !x.s.alive || !j.on || j.phase != 2 {
        return None;
    }
    let p = j.p;
    let violates = !x.g.guard || (x.s.disk[p] != j.base && !x.config.r1);
    let unseen = x.g.guard
        && j.bytes != j.base
        && !(x.g.seen[p].contains(&j.base)
            || x.g.wrote[p].iter().any(|w| w.0 == j.base)
            || x.g.op_read[p].contains(&j.base));
    let mut s = x.s.clone();
    let mut g = x.g.clone();
    let del = j.bytes == ABSENT && x.s.disk[p] != ABSENT;
    if del {
        if !x.config.mutant("MDT") {
            s.trash[p].insert(x.s.disk[p]);
        }
        g.removed[p].insert(x.s.disk[p]);
    }
    s.disk[p] = j.bytes;
    s.job.phase = 3;
    s.job.ep = g.ext[p];
    if x.config.mutant("MPR") {
        discharge(&mut s, p, j.ver);
    }
    g.guard = false;
    g.wrote[p].insert((j.bytes, j.ver));
    if violates {
        g.bad.insert("C-overwrote-external".into());
    }
    if unseen {
        g.bad.insert("G-overwrote-unseen".into());
    }
    commit(x, s, g, "rename")
}
/// Quint: dirSync
pub(super) fn dir_sync(x: &State, ok: bool) -> Option<State> {
    let j = &x.s.job;
    if !x.s.alive || !j.on || j.phase != 3 {
        return None;
    }
    let p = j.p;
    let mut s = x.s.clone();
    let mut g = x.g.clone();
    if ok && !x.config.weak && s.disk[p] == j.bytes {
        s.stable[p] = s.disk[p];
        if !x.config.mutant("MTS") {
            s.trash_stable[p] = s.trash[p].clone();
        }
        if j.bytes == ABSENT {
            g.del_durable[p] = g.removed[p].clone();
        }
    }
    s.pages[p] = if !ok && x.config.mutant("MRE") {
        Page {
            base: j.bytes,
            typed: false,
            risk: false,
            ..s.pages[p].clone()
        }
    } else {
        on_reply(
            x,
            &s.pages[p],
            if ok { "Published" } else { "Uncertain" },
            j.bytes,
        )
    };
    s.job = nojob();
    if ok {
        g.promise[p] = ack(&g.promise[p], j.bytes, j.ver, true, j.ep);
        obl_published(&mut g.obl, p, j.ver);
        published_pair(x, &mut s, &mut g, p, j.bytes);
    }
    // s3.2: Published is the witness (MPR took it at the file step; MCV at any end).
    if (ok && !x.config.mutant("MPR")) || x.config.mutant("MCV") {
        discharge(&mut s, p, j.ver);
    }
    commit(x, s, g, "dirSync")
}
/// Quint: saveFail
pub(super) fn save_fail(x: &State) -> Option<State> {
    let j = &x.s.job;
    if !x.s.alive || !j.on || j.phase > 2 {
        return None;
    }
    let mut s = x.s.clone();
    let mut g = x.g.clone();
    s.pages[j.p] = on_reply(x, &s.pages[j.p], "Failed", j.bytes);
    s.job = nojob();
    g.guard = false;
    if x.config.mutant("MCV") {
        discharge(&mut s, j.p, j.ver);
    }
    commit(x, s, g, "saveFail")
}
/// Quint: draftSync
pub(super) fn draft_sync(x: &State, p: usize) -> Option<State> {
    let mut e = wanted_draft(x, p);
    if x.config.mutant("MBD") && e.base == UNKNOWN {
        e.base = x.s.disk[p];
    }
    if !x.s.alive || !can_draft(&x.s) || x.s.drafts[p] == e {
        return None;
    }
    let mut s = x.s.clone();
    let mut g = x.g.clone();
    if e.bytes != NONE {
        g.wrote[p].insert((e.bytes, e.ver));
        g.promise[p] = ack(&g.promise[p], e.bytes, e.ver, false, 0);
    }
    s.drafts[p] = e;
    commit(x, s, g, "draftWrite")
}
