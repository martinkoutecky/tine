//! Executable, unwired transcription of storage-s3.qnt. See README.md.
#![allow(dead_code)]

#[cfg(test)]
use operations::op_clean;
use operations::{flush_del, op_delete, op_rename};
use std::collections::BTreeSet;

mod operations;

const ABSENT: i64 = -1;
const NONE: i64 = -2;
const UNKNOWN: i64 = -3;
const MODEL_SHA: &str = "baaaeab459890ea8b09c49dbd0ab506489c372c944c71aec8b12f43e3ebba557";
type Text = i64; // Opaque equality labels, never arithmetic operands.
type Pairs = BTreeSet<(usize, Text)>;

macro_rules! record {
    ($name:ident { $($(#[$attr:meta])* $field:ident: $ty:ty),* $(,)? }) => {
        #[derive(Clone, Debug, PartialEq, Eq)]
        #[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
        struct $name { $( $(#[$attr])* $field: $ty, )* }
    };
}
record!(Page {
    held: bool,
    buf: Text,
    base: Text,
    ver: i64,
    risk: bool,
    conflict: bool,
    typed: bool,
    obs: Text
});
record!(Draft {
    bytes: Text,
    base: Text,
    ver: i64
});
record!(Job {
    on: bool,
    p: usize,
    bytes: Text,
    base: Text,
    ver: i64,
    phase: i64,
    ep: i64
});
record!(W {
    on: bool,
    text: Text,
    bv: i64,
    pend: bool,
    sent: bool,
    obs: Text,
    conf: bool
});
record!(Mail {
    on: bool,
    held: bool,
    ver: i64,
    text: Text,
    obs: Text,
    conf: bool,
    ack: i64,
    took: bool
});
record!(Up {
    kind: String,
    p: usize,
    q: usize,
    t: Text,
    t2: Text,
    bv: i64,
    bv2: i64,
    ro: Text,
    cur: bool
});
record!(Sys { alive: bool, disk: Vec<Text>, stable: Vec<Text>, drafts: Vec<Draft>, trash: Vec<BTreeSet<Text>>, #[cfg_attr(test, serde(rename = "trashStable"))] trash_stable: Vec<BTreeSet<Text>>, pages: Vec<Page>, job: Job, w: Vec<W>, mb: Vec<Mail>, up: Vec<Up> });
record!(Promise {
    on: bool,
    bytes: Text,
    ver: i64,
    saved: bool,
    ep: i64
});
record!(Ghost { vc: i64, promise: Vec<Promise>, ext: Vec<i64>, wrote: Vec<BTreeSet<(Text, i64)>>, owed: Pairs, guard: bool, seen: Vec<BTreeSet<Text>>, mine: BTreeSet<usize>, #[cfg_attr(test, serde(rename = "opRead"))] op_read: Vec<BTreeSet<Text>>, removed: Vec<BTreeSet<Text>>, #[cfg_attr(test, serde(rename = "delDurable"))] del_durable: Vec<BTreeSet<Text>>, bad: BTreeSet<String> });

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Config {
    pages: usize,
    r1: bool,
    weak: bool,
    #[cfg(test)]
    mutant: String,
}
impl Config {
    fn mutant(&self, name: &str) -> bool {
        #[cfg(test)]
        {
            self.mutant == name
        }
        #[cfg(not(test))]
        {
            let _ = name;
            false
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
struct State {
    s: Sys,
    g: Ghost,
    #[cfg_attr(test, serde(skip))]
    config: Config,
}

fn nopage() -> Page {
    Page {
        held: false,
        buf: ABSENT,
        base: ABSENT,
        ver: 0,
        risk: false,
        conflict: false,
        typed: false,
        obs: ABSENT,
    }
}
fn nodraft() -> Draft {
    Draft {
        bytes: NONE,
        base: NONE,
        ver: 0,
    }
}
fn nojob() -> Job {
    Job {
        on: false,
        p: 0,
        bytes: 0,
        base: 0,
        ver: 0,
        phase: 0,
        ep: 0,
    }
}
fn now() -> W {
    W {
        on: false,
        text: ABSENT,
        bv: -1,
        pend: false,
        sent: false,
        obs: ABSENT,
        conf: false,
    }
}
fn nomail() -> Mail {
    Mail {
        on: false,
        held: false,
        ver: 0,
        text: ABSENT,
        obs: ABSENT,
        conf: false,
        ack: -1,
        took: false,
    }
}
fn nopromise() -> Promise {
    Promise {
        on: false,
        bytes: NONE,
        ver: 0,
        saved: false,
        ep: 0,
    }
}
fn next(v: i64) -> i64 {
    v.checked_add(1)
        .expect("s3 counter exceeds Rust i64 representation")
}
fn real(v: Text) -> bool {
    [1, 2, 3].contains(&v)
}

/// Quint: init
fn init(config: Config) -> State {
    assert!(config.pages > 0, "PAGES must be nonempty");
    let count = config.pages;
    let disk: Vec<_> = (0..count)
        .map(|p| match p {
            0 => 1,
            1 => 2,
            _ => ABSENT,
        })
        .collect();
    State {
        config,
        s: Sys {
            alive: true,
            disk: disk.clone(),
            stable: disk,
            drafts: (0..count).map(|_| nodraft()).collect(),
            pages: (0..count).map(|_| nopage()).collect(),
            job: nojob(),
            w: (0..count).map(|_| now()).collect(),
            mb: (0..count).map(|_| nomail()).collect(),
            trash: vec![BTreeSet::new(); count],
            trash_stable: vec![BTreeSet::new(); count],
            up: vec![],
        },
        g: Ghost {
            vc: 0,
            promise: (0..count).map(|_| nopromise()).collect(),
            ext: vec![0; count],
            wrote: (0..count).map(|_| BTreeSet::new()).collect(),
            owed: BTreeSet::new(),
            guard: false,
            seen: (0..count).map(|_| BTreeSet::new()).collect(),
            mine: BTreeSet::new(),
            op_read: vec![BTreeSet::new(); count],
            removed: vec![BTreeSet::new(); count],
            del_durable: vec![BTreeSet::new(); count],
            bad: BTreeSet::new(),
        },
    }
}
/// Quint: clean
fn clean(pg: &Page) -> bool {
    !pg.typed && pg.buf == pg.base && !pg.risk
}
/// Quint: dirty
fn dirty(pg: &Page) -> bool {
    !clean(pg)
}
/// Quint: draftEntry
fn draft_entry(pg: &Page) -> Draft {
    if pg.held && pg.risk {
        Draft {
            bytes: pg.buf,
            base: pg.base,
            ver: pg.ver,
        }
    } else {
        nodraft()
    }
}
/// Quint: table
fn table(x: &State, pg: &Page, v: Text, nv: i64) -> Page {
    let mut q = pg.clone();
    q.obs = v;
    if x.config.mutant("MQ") && v == q.buf {
        q.base = v;
        q.conflict = false;
        q.risk = false;
        q.typed = false;
    } else if v == q.base {
        q.conflict = false;
    } else if clean(&q) || (x.config.mutant("MLO") && !q.risk) {
        q.buf = v;
        q.base = v;
        q.ver = nv;
        q.conflict = false;
        q.typed = false;
    } else if v == q.buf {
        q.base = v;
        q.conflict = false;
    } else {
        q.conflict = true;
        q.risk = true;
    }
    q
}
/// Quint: onReply
fn on_reply(x: &State, pg: &Page, out: &str, bytes: Text) -> Page {
    let mut q = pg.clone();
    if out == "Published" || (out == "Uncertain" && x.config.mutant("MLS")) {
        q.base = bytes;
        q.typed = false;
        q.risk = false;
    } else {
        q.risk = true;
    }
    q
}
/// Quint: submitTo
fn submit_to(x: &State, pg: &Page, t: Text, bv: i64, v: i64) -> Page {
    let mut q = pg.clone();
    if x.config.mutant("MIS") {
        return q;
    }
    q.buf = t;
    q.typed = true;
    q.ver = v;
    if bv != pg.ver && !x.config.mutant("MST") {
        q.base = UNKNOWN;
        q.conflict = true;
        q.risk = true;
    }
    q
}
/// Quint: ack
fn ack(pr: &Promise, bytes: Text, ver: i64, saved: bool, ep: i64) -> Promise {
    if ver > pr.ver || (ver == pr.ver && pr.on && saved && !pr.saved) {
        Promise {
            on: true,
            bytes,
            ver,
            saved,
            ep,
        }
    } else {
        pr.clone()
    }
}
/// Quint: endPromise
fn end_promise(pr: &Promise, watermark: i64) -> Promise {
    Promise {
        on: false,
        ver: watermark,
        ..pr.clone()
    }
}
/// Quint: down
fn down(sys: &Sys) -> Sys {
    Sys {
        alive: false,
        pages: vec![nopage(); sys.pages.len()],
        job: nojob(),
        w: vec![now(); sys.pages.len()],
        mb: vec![nomail(); sys.pages.len()],
        up: vec![],
        ..sys.clone()
    }
}
/// Quint: gDown
fn g_down(g: &Ghost) -> Ghost {
    Ghost {
        owed: BTreeSet::new(),
        guard: false,
        ..g.clone()
    }
}
/// Quint: locked
fn locked(x: &State, p: usize) -> bool {
    x.s.job.on && x.s.job.p == p
}
/// Quint: ackMail
fn ack_mail(sys: &mut Sys, p: usize, ok: bool, v: i64, took: bool) {
    if ok {
        let pg = &sys.pages[p];
        sys.mb[p] = Mail {
            on: true,
            held: pg.held,
            ver: pg.ver,
            text: pg.buf,
            obs: pg.obs,
            conf: pg.conflict,
            ack: v,
            took,
        };
    }
}
/// Quint: up1
fn up1(kind: &str, p: usize, t: Text, bv: i64, ro: Text) -> Up {
    Up {
        kind: kind.into(),
        p,
        q: 0,
        t,
        t2: 0,
        bv,
        bv2: 0,
        ro,
        cur: true,
    }
}
/// Quint: carries
fn carries(u: &Up) -> Pairs {
    match u.kind.as_str() {
        "submit" => BTreeSet::from([(u.p, u.t)]),
        "op" => BTreeSet::from([(u.p, u.t), (u.q, u.t2)]),
        _ => BTreeSet::new(),
    }
}
/// Quint: keptB (B, evaluated using the post-commit promises and pre-step seen set).
fn kept_b(
    x: &State,
    p: usize,
    ns: &Sys,
    np: &[Promise],
    mine: &BTreeSet<usize>,
    name: &str,
) -> bool {
    let a = &x.s.pages[p];
    let b = &ns.pages[p];
    let pr = &np[p];
    let replaced = a.held && (!b.held || b.buf != a.buf);
    let safe = (!a.typed && a.buf == a.obs) || (pr.ver >= a.ver && pr.bytes == a.buf);
    let shown =
        ["uStale", "uOpStale"].contains(&name) && (x.g.seen[p].contains(&a.buf) || !a.typed);
    !replaced
        || mine.contains(&p)
        || ["crash", "power"].contains(&name)
        || safe
        || shown
        || (name == "switchFin" && ns.drafts[p].bytes == a.buf)
}
/// Quint: keptW (B′(1)).
fn kept_w(x: &State, p: usize, ns: &Sys, owed: &Pairs, name: &str) -> bool {
    let a = &x.s.w[p];
    let b = &ns.w[p];
    let replaced = a.on && (!b.on || b.text != a.text);
    !replaced
        || ["wEdit", "wResolve", "wOp", "windowCrash", "crash", "power"].contains(&name)
        || (!a.pend && (!a.sent || (name == "wAck" && !owed.contains(&(p, a.text)))))
}
/// Quint: commit
fn commit(x: &State, mut ns: Sys, mut ng: Ghost, name: &str) -> Option<State> {
    let np: Vec<_> = (0..x.s.pages.len())
        .map(|p| {
            let a = &x.s.pages[p];
            let b = &ns.pages[p];
            if !ng.mine.contains(&p) && a.held && dirty(a) && b.held && clean(b) && b.buf == a.buf {
                ack(&ng.promise[p], b.buf, b.ver, false, 0)
            } else {
                ng.promise[p].clone()
            }
        })
        .collect();
    for p in 0..x.s.pages.len() {
        let a = &x.s.pages[p];
        let b = &ns.pages[p];
        let m = &ns.mb[p];
        let w = &ns.w[p];
        if ns.alive
            && (w.on || w.sent)
            && (a.ver != b.ver
                || a.buf != b.buf
                || a.obs != b.obs
                || a.conflict != b.conflict
                || a.held != b.held)
        {
            ns.mb[p] = Mail {
                on: true,
                held: b.held,
                ver: b.ver,
                text: b.buf,
                obs: b.obs,
                conf: b.conflict,
                ack: if m.on { m.ack } else { -1 },
                took: m.on && m.took,
            };
        }
        if !kept_b(x, p, &ns, &np, &ng.mine, name) {
            ng.bad.insert("B-unsaved-replaced".into());
        }
        if !kept_w(x, p, &ns, &ng.owed, name) {
            ng.bad.insert("B'-window-replaced".into());
        }
        if w.on && (w.text == ABSENT || real(w.text)) {
            ng.seen[p].insert(w.text);
        }
    }
    ng.promise = np;
    ng.mine.clear();
    Some(State {
        s: ns,
        g: ng,
        config: x.config.clone(),
    })
}

/// Quint: wOpen
fn w_open(x: &State, p: usize) -> Option<State> {
    let a = &x.s.w[p];
    if !x.s.alive || a.on || a.sent {
        return None;
    }
    let mut s = x.s.clone();
    s.w[p] = W {
        sent: true,
        ..now()
    };
    s.up.push(up1("open", p, 0, 0, NONE));
    commit(x, s, x.g.clone(), "wOpen")
}
/// Quint: wEdit
fn w_edit(x: &State, p: usize, v: Text) -> Option<State> {
    let a = &x.s.w[p];
    if !x.s.alive || !a.on || v == a.text {
        return None;
    }
    let mut s = x.s.clone();
    s.w[p].text = v;
    s.w[p].pend = true;
    commit(x, s, x.g.clone(), "wEdit")
}
/// Quint: wSend
fn w_send(x: &State, p: usize) -> Option<State> {
    let a = &x.s.w[p];
    if !x.s.alive || !a.on || !a.pend || a.sent {
        return None;
    }
    let mut s = x.s.clone();
    let mut g = x.g.clone();
    s.w[p].pend = false;
    s.w[p].sent = true;
    s.up.push(up1("submit", p, a.text, a.bv, NONE));
    g.owed.insert((p, a.text));
    commit(x, s, g, "wSend")
}
/// Quint: wResolve
fn w_resolve(x: &State, p: usize, v: Text) -> Option<State> {
    let a = &x.s.w[p];
    if !x.s.alive || !a.on || !a.conf || a.sent {
        return None;
    }
    let mut s = x.s.clone();
    let mut g = x.g.clone();
    s.w[p].text = v;
    s.w[p].pend = false;
    s.w[p].sent = true;
    s.up.push(up1("submit", p, v, a.bv, a.obs));
    g.owed.insert((p, v));
    if a.obs == ABSENT || real(a.obs) {
        g.seen[p].insert(a.obs);
    }
    commit(x, s, g, "wResolve")
}
/// Quint: wDiscard
fn w_discard(x: &State, p: usize) -> Option<State> {
    let a = &x.s.w[p];
    if !x.s.alive || !a.on || a.sent {
        return None;
    }
    let mut s = x.s.clone();
    s.w[p].pend = false;
    s.w[p].sent = true;
    s.up.push(up1("discard", p, 0, a.bv, NONE));
    commit(x, s, x.g.clone(), "wDiscard")
}
/// Quint: wOp
fn w_op(x: &State, src: usize, ds: Text, dd: Text) -> Option<State> {
    let dst = 1usize.checked_sub(src)?;
    if dst >= x.s.pages.len() {
        return None;
    }
    w_op_to(x, src, dst, ds, dd)
}
/// Quint: wOpTo
fn w_op_to(x: &State, src: usize, dst: usize, ds: Text, dd: Text) -> Option<State> {
    let a = &x.s.w[src];
    let b = &x.s.w[dst];
    if !x.s.alive
        || src == dst
        || !a.on
        || !b.on
        || a.pend
        || a.sent
        || b.pend
        || b.sent
        || ds == a.text
        || dd == b.text
    {
        return None;
    }
    let mut s = x.s.clone();
    let mut g = x.g.clone();
    s.w[src].text = ds;
    s.w[src].sent = true;
    s.w[dst].text = dd;
    s.w[dst].sent = true;
    s.up.push(Up {
        kind: "op".into(),
        p: src,
        q: dst,
        t: ds,
        t2: dd,
        bv: a.bv,
        bv2: b.bv,
        ro: NONE,
        cur: true,
    });
    g.owed.extend([(src, ds), (dst, dd)]);
    commit(x, s, g, "wOp")
}
/// Quint: wClose
fn w_close(x: &State, p: usize) -> Option<State> {
    let a = &x.s.w[p];
    if !x.s.alive || !a.on || a.pend || a.sent {
        return None;
    }
    let mut s = x.s.clone();
    s.w[p] = now();
    s.up.push(up1("close", p, 0, 0, NONE));
    commit(x, s, x.g.clone(), "wClose")
}
/// Quint: wRecv
fn w_recv(x: &State, p: usize) -> Option<State> {
    let m = &x.s.mb[p];
    let a = &x.s.w[p];
    if !x.s.alive || !m.on {
        return None;
    }
    let is_ack = (m.ack >= 0 || x.config.mutant("MAK")) && a.sent;
    let mut na = a.clone();
    if is_ack && !m.held {
        na = now();
    } else if is_ack && a.pend {
        na.sent = false;
        na.bv = if m.took || x.config.mutant("MDA") {
            m.ack
        } else {
            a.bv
        };
        na.obs = m.obs;
        na.conf = m.conf;
    } else if is_ack {
        na = W {
            on: true,
            text: m.text,
            bv: m.ver,
            pend: false,
            sent: false,
            obs: m.obs,
            conf: m.conf,
        };
    } else if a.on && !a.sent && (!a.pend || x.config.mutant("MWA")) {
        na.text = m.text;
        na.bv = m.ver;
        na.obs = m.obs;
        na.conf = m.conf;
    } else if a.on {
        na.obs = m.obs;
        na.conf = m.conf;
    }
    let mut s = x.s.clone();
    s.w[p] = na;
    s.mb[p] = nomail();
    commit(x, s, x.g.clone(), if is_ack { "wAck" } else { "wRecv" })
}

/// Quint: upOpen
fn up_open(x: &State, mut s: Sys, m: &Up, ok: bool, rok: bool) -> Option<State> {
    let pg = s.pages[m.p].clone();
    let v1 = next(x.g.vc);
    let mut g = x.g.clone();
    let np = if pg.held || !rok {
        pg.clone()
    } else {
        Page {
            held: true,
            buf: x.s.disk[m.p],
            base: x.s.disk[m.p],
            obs: x.s.disk[m.p],
            ver: v1,
            ..nopage()
        }
    };
    if np.ver == v1 {
        g.vc = v1;
    }
    let ver = np.ver;
    s.pages[m.p] = np;
    ack_mail(&mut s, m.p, ok, ver, false);
    commit(x, s, g, if pg.held || rok { "uOpen" } else { "uOpenFail" })
}
/// Quint: upClose
fn up_close(x: &State, mut s: Sys, m: &Up) -> Option<State> {
    let pg = &s.pages[m.p];
    if pg.held && (clean(pg) || (x.config.mutant("MCL") && !pg.risk)) {
        s.pages[m.p] = nopage();
    }
    commit(x, s, x.g.clone(), "uClose")
}
/// Quint: upDiscard
fn up_discard(x: &State, mut s: Sys, m: &Up, ok: bool, rok: bool) -> Option<State> {
    let p = m.p;
    let pg = s.pages[p].clone();
    let d = x.s.disk[p];
    let v1 = next(x.g.vc);
    let mut g = x.g.clone();
    let pr = &g.promise[p];
    let hold = pg.risk && (d == pg.buf || d == x.s.drafts[p].bytes) && !x.config.mutant("MDE");
    let live = pr.on && !(pr.saved && g.ext[p] > pr.ep);
    let npr = if live && pr.bytes == d {
        pr.clone()
    } else {
        end_promise(pr, v1)
    };
    if !pg.held {
        g.bad.insert("request-to-unheld".into());
    } else if !rok {
        ack_mail(&mut s, p, ok, pg.ver, false);
    } else {
        s.pages[p] = Page {
            buf: d,
            base: d,
            ver: v1,
            obs: d,
            conflict: false,
            risk: hold,
            typed: false,
            ..pg
        };
        ack_mail(&mut s, p, ok, v1, false);
        g.vc = v1;
        g.promise[p] = npr;
        g.mine = BTreeSet::from([p]);
    }
    commit(
        x,
        s,
        g,
        if pg.held && !rok {
            "uDiscardFail"
        } else {
            "uDiscard"
        },
    )
}
/// Quint: upSubmit
fn up_submit(x: &State, mut s: Sys, m: &Up, ok: bool) -> Option<State> {
    let p = m.p;
    let pg = s.pages[p].clone();
    let v1 = next(x.g.vc);
    let mut g = x.g.clone();
    let resolve = m.ro != NONE;
    let nw = if resolve && !x.config.mutant("MIS") {
        Page {
            buf: m.t,
            base: m.ro,
            typed: true,
            ver: v1,
            conflict: pg.obs != m.ro,
            risk: pg.risk || pg.obs != m.ro,
            ..pg.clone()
        }
    } else {
        submit_to(x, &pg, m.t, m.bv, v1)
    };
    let stale = !resolve && m.bv != pg.ver;
    if !pg.held {
        g.bad.insert("request-to-unheld".into());
    } else {
        let took = nw.buf == m.t;
        s.pages[p] = nw;
        if x.config.mutant("MDO") {
            for q in 0..s.pages.len() {
                if q != p {
                    s.pages[q] = nopage();
                }
            }
        }
        ack_mail(&mut s, p, ok, v1, took);
        g.vc = v1;
        g.mine = if stale {
            BTreeSet::new()
        } else {
            BTreeSet::from([p])
        };
        g.owed.remove(&(p, m.t));
        if !took {
            g.bad.insert("B'-submit-not-taken".into());
        }
    }
    commit(
        x,
        s,
        g,
        if !pg.held {
            "uSubmit"
        } else if resolve {
            "uResolve"
        } else if stale {
            "uStale"
        } else {
            "uSubmit"
        },
    )
}
/// Quint: upOp
fn up_op(x: &State, mut s: Sys, m: &Up, ok: bool, dok: bool) -> Option<State> {
    let src = m.p;
    let dst = m.q;
    let a = s.pages[src].clone();
    let b = s.pages[dst].clone();
    let vd = next(x.g.vc);
    let vs = next(vd);
    let mut g = x.g.clone();
    let mut nb = submit_to(x, &b, m.t2, m.bv2, vd);
    nb.risk = true;
    let na = submit_to(x, &a, m.t, m.bv, vs);
    let stale: BTreeSet<_> = [src, dst]
        .into_iter()
        .filter(|p| {
            if *p == src {
                m.bv != a.ver
            } else {
                m.bv2 != b.ver
            }
        })
        .collect();
    let name;
    if !a.held || !b.held {
        g.bad.insert("request-to-unheld".into());
        name = "uOp";
    } else {
        g.owed.remove(&(src, m.t));
        g.owed.remove(&(dst, m.t2));
        if (!dok && !x.config.mutant("MOD")) || (!stale.is_empty() && !x.config.mutant("MSM")) {
            ack_mail(&mut s, dst, ok, b.ver, false);
            ack_mail(&mut s, src, ok, a.ver, false);
            name = "uOpRefused";
        } else {
            if !x.config.mutant("MOD") {
                s.drafts[dst] = draft_entry(&nb);
            }
            let took_d = nb.buf == m.t2;
            let took_s = na.buf == m.t;
            s.pages[dst] = nb;
            s.pages[src] = na;
            ack_mail(&mut s, dst, ok, vd, took_d);
            ack_mail(&mut s, src, ok, vs, took_s);
            g.vc = vs;
            g.mine = BTreeSet::from([src, dst])
                .difference(&stale)
                .copied()
                .collect();
            g.wrote[dst].insert((m.t2, vd));
            g.promise[dst] = ack(&g.promise[dst], m.t2, vd, false, 0);
            if !took_d || !took_s {
                g.bad.insert("B'-submit-not-taken".into());
            }
            name = if stale.is_empty() { "uOp" } else { "uOpStale" };
        }
    }
    commit(x, s, g, name)
}
/// Quint: deliverUp
fn deliver_up(x: &State, rok: bool) -> Option<State> {
    if !x.s.alive || x.s.up.is_empty() {
        return None;
    }
    let m = x.s.up[0].clone();
    if locked(x, m.p) || (m.kind == "op" && locked(x, m.q)) {
        return None;
    }
    let mut s = x.s.clone();
    s.up.remove(0);
    match m.kind.as_str() {
        "open" => up_open(x, s, &m, m.cur, rok),
        "close" => up_close(x, s, &m),
        "discard" => up_discard(x, s, &m, m.cur, rok),
        "op" => up_op(x, s, &m, m.cur, rok),
        _ => up_submit(x, s, &m, m.cur),
    }
}

/// Quint: observe
fn observe(x: &State, p: usize) -> Option<State> {
    let pg = &x.s.pages[p];
    let d = x.s.disk[p];
    let v1 = next(x.g.vc);
    let nw = table(x, pg, d, v1);
    if !x.s.alive || !pg.held || locked(x, p) || nw == *pg {
        return None;
    }
    let mut s = x.s.clone();
    let mut g = x.g.clone();
    if nw.ver == v1 {
        g.vc = v1;
    }
    s.pages[p] = nw;
    commit(x, s, g, "observe")
}
/// Quint: flush
fn flush(x: &State, p: usize) -> Option<State> {
    let pg = &x.s.pages[p];
    if !x.s.alive || x.s.job.on || !pg.held || pg.conflict || pg.buf == ABSENT || !dirty(pg) {
        return None;
    }
    let mut s = x.s.clone();
    s.job = Job {
        on: true,
        p,
        bytes: pg.buf,
        base: pg.base,
        ver: pg.ver,
        phase: 1,
        ep: 0,
    };
    commit(x, s, x.g.clone(), "flush")
}
/// Quint: check
fn check(x: &State) -> Option<State> {
    let j = &x.s.job;
    if !x.s.alive || !j.on || j.phase != 1 {
        return None;
    }
    let p = j.p;
    let d = x.s.disk[p];
    let mut s = x.s.clone();
    let mut g = x.g.clone();
    let name = if d != j.base && !x.config.mutant("MC") && !x.config.mutant("MC1") {
        let v1 = next(g.vc);
        let nw = table(x, &s.pages[p], d, v1);
        if nw.ver == v1 {
            g.vc = v1;
        }
        s.pages[p] = nw;
        s.job = nojob();
        "mismatch"
    } else {
        s.job.phase = 2;
        g.guard = d == j.base;
        "check"
    };
    commit(x, s, g, name)
}
/// Quint: rename
fn rename(x: &State) -> Option<State> {
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
fn dir_sync(x: &State, ok: bool) -> Option<State> {
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
    }
    commit(x, s, g, "dirSync")
}
/// Quint: saveFail
fn save_fail(x: &State) -> Option<State> {
    let j = &x.s.job;
    if !x.s.alive || !j.on || j.phase > 2 {
        return None;
    }
    let mut s = x.s.clone();
    let mut g = x.g.clone();
    s.pages[j.p] = on_reply(x, &s.pages[j.p], "Failed", j.bytes);
    s.job = nojob();
    g.guard = false;
    commit(x, s, g, "saveFail")
}
/// Quint: draftSync
fn draft_sync(x: &State, p: usize) -> Option<State> {
    let mut e = draft_entry(&x.s.pages[p]);
    if x.config.mutant("MBD") && e.base == UNKNOWN {
        e.base = x.s.disk[p];
    }
    if !x.s.alive || x.s.drafts[p] == e {
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
/// Quint: switchReq
fn switch_req(x: &State) -> Option<State> {
    if !x.s.alive || !x.s.pages.iter().any(|pg| pg.held && dirty(pg) && !pg.risk) {
        return None;
    }
    let mut s = x.s.clone();
    for pg in &mut s.pages {
        if pg.held && dirty(pg) {
            pg.risk = true;
        }
    }
    commit(x, s, x.g.clone(), "switchReq")
}
/// Quint: canSwitch
fn can_switch(x: &State) -> bool {
    x.s.alive
        && !x.s.job.on
        && x.s.up.is_empty()
        && (0..x.s.pages.len()).all(|p| {
            let pg = &x.s.pages[p];
            let a = &x.s.w[p];
            !a.pend
                && !a.sent
                && (!pg.held || clean(pg) || pg.risk)
                && (x.s.drafts[p] == draft_entry(pg)
                    || (x.config.mutant("MX") && pg.held && pg.risk))
        })
}
/// Quint: switchFin
fn switch_fin(x: &State) -> Option<State> {
    if !can_switch(x) {
        None
    } else {
        commit(x, down(&x.s), g_down(&x.g), "switchFin")
    }
}
/// Quint: window
fn window(s: &Sys, p: usize) -> bool {
    s.job.on && s.job.phase == 2 && s.job.p == p
}
/// Quint: extWriteD
fn ext_write_d(x: &State, p: usize, v: Text, dur: bool) -> Option<State> {
    if v == x.s.disk[p] || (window(&x.s, p) && !x.config.r1) {
        return None;
    }
    let mut s = x.s.clone();
    let mut g = x.g.clone();
    s.disk[p] = v;
    if dur {
        s.stable[p] = v;
    }
    g.ext[p] = next(g.ext[p]);
    commit(x, s, g, "extWrite")
}
/// Quint: extWrite
fn ext_write(x: &State, p: usize, v: Text) -> Option<State> {
    ext_write_d(x, p, v, true)
}
/// Quint: crash
fn crash(x: &State) -> Option<State> {
    if !x.s.alive {
        None
    } else {
        commit(x, down(&x.s), g_down(&x.g), "crash")
    }
}
/// Quint: windowCrash
fn window_crash(x: &State) -> Option<State> {
    if !x.s.alive {
        return None;
    }
    let mut s = x.s.clone();
    s.w = vec![now(); x.s.pages.len()];
    s.mb = vec![nomail(); x.s.pages.len()];
    if x.config.mutant("MDQ") {
        s.up.clear();
    } else {
        for u in &mut s.up {
            u.cur = false;
        }
    }
    commit(x, s, x.g.clone(), "windowCrash")
}
/// Quint: power
fn power(x: &State, keep0: bool, keep1: bool) -> Option<State> {
    let keep = (0..x.s.pages.len())
        .filter(|&p| (p == 0 && keep0) || (p == 1 && keep1))
        .collect();
    power_k(x, &keep)
}
/// Quint: powerK
fn power_k(x: &State, keep: &BTreeSet<usize>) -> Option<State> {
    let nd: Vec<_> = (0..x.s.pages.len())
        .map(|p| {
            if keep.contains(&p) {
                x.s.disk[p]
            } else {
                x.s.stable[p]
            }
        })
        .collect();
    let mut s = down(&x.s);
    let mut g = g_down(&x.g);
    for p in 0..x.s.pages.len() {
        let pr = &g.promise[p];
        let kept = [nd[p], x.s.drafts[p].bytes].contains(&pr.bytes);
        if x.config.weak && pr.on && pr.saved && !kept {
            g.promise[p] = end_promise(pr, pr.ver);
        }
        s.trash[p] = if keep.contains(&p) {
            x.s.trash[p].clone()
        } else {
            x.s.trash_stable[p].clone()
        };
        s.trash_stable[p] = s.trash[p].clone();
        g.removed[p] = if keep.contains(&p) {
            x.g.removed[p].clone()
        } else {
            x.g.del_durable[p].clone()
        };
        g.del_durable[p] = x.g.del_durable[p]
            .intersection(&g.removed[p])
            .copied()
            .collect();
    }
    s.disk = nd.clone();
    s.stable = nd;
    commit(x, s, g, "power")
}
/// Quint: restored
fn restored(x: &State) -> Vec<Page> {
    (0..x.s.pages.len())
        .map(|p| {
            let d = &x.s.drafts[p];
            if d.bytes == NONE {
                nopage()
            } else {
                Page {
                    held: true,
                    buf: d.bytes,
                    base: if x.config.mutant("MRB") && d.base == UNKNOWN {
                        x.s.disk[p]
                    } else {
                        d.base
                    },
                    obs: NONE,
                    ver: (0..p)
                        .filter(|&q| x.s.drafts[q].bytes != NONE)
                        .fold(next(x.g.vc), |v, _| next(v)),
                    risk: true,
                    typed: true,
                    ..nopage()
                }
            }
        })
        .collect()
}
/// Quint: launch
fn launch(x: &State) -> Option<State> {
    if x.s.alive {
        return None;
    }
    let mut s = x.s.clone();
    let mut g = x.g.clone();
    s.alive = true;
    s.pages = restored(x);
    for d in &s.drafts {
        if d.bytes != NONE {
            g.vc = next(g.vc);
        }
    }
    commit(x, s, g, "launch")
}
/// Quint: copies
fn copies(x: &State, p: usize) -> BTreeSet<Text> {
    BTreeSet::from([x.s.disk[p], x.s.drafts[p].bytes])
}
/// Quint: noLoss (A).
fn no_loss(x: &State) -> bool {
    x.s.alive
        || (0..x.s.pages.len()).all(|p| {
            let pr = &x.g.promise[p];
            !pr.on
                || (pr.saved && x.g.ext[p] > pr.ep)
                || copies(x, p).iter().any(|&c| {
                    c == pr.bytes || x.g.wrote[p].iter().any(|w| w.0 == c && w.1 > pr.ver)
                })
        })
}
/// Quint: accepted (B′(3)).
fn accepted(x: &State) -> bool {
    !x.s.alive
        || x.g
            .owed
            .iter()
            .all(|o| x.s.up.iter().any(|u| carries(u).contains(o)))
}
/// Quint: transitions (B, B′(1/2), C, G and request-to-unheld).
fn transitions(x: &State) -> bool {
    x.g.bad.is_empty()
}
/// Quint: trashed (D).
fn trashed(x: &State) -> bool {
    (0..x.s.pages.len()).all(|p| {
        x.g.removed[p]
            .iter()
            .all(|b| x.s.trash[p].contains(b) || x.s.disk[p] == *b)
    })
}
/// Quint: guarantee
fn guarantee(x: &State) -> bool {
    no_loss(x) && transitions(x) && accepted(x) && trashed(x)
}
/// Quint: C, history recorded by rename.
fn clause_c(x: &State) -> bool {
    !x.g.bad.contains("C-overwrote-external")
}
/// Quint: G, history recorded by rename.
fn clause_g(x: &State) -> bool {
    !x.g.bad.contains("G-overwrote-unseen")
}
/// Quint: B, history recorded by commit.
fn clause_b(x: &State) -> bool {
    !x.g.bad.contains("B-unsaved-replaced")
}
/// Quint: B′, history recorded by commit and upSubmit/upOp, plus accepted.
fn clause_b_prime(x: &State) -> bool {
    !x.g.bad.contains("B'-window-replaced")
        && !x.g.bad.contains("B'-submit-not-taken")
        && accepted(x)
}
/// Quint: within (diagnostic, never a guard).
fn within(x: &State) -> bool {
    x.g.vc <= 1000 && x.g.ext.iter().all(|v| *v <= 1000) && x.s.up.len() <= 1000
}

// Explicit choices replace step's nondeterministic parameter selection.
#[derive(Clone, Debug)]
enum Action {
    WOpen(usize),
    WEdit(usize, Text),
    WSend(usize),
    WResolve(usize, Text),
    WDiscard(usize),
    WOp(usize, Text, Text),
    WOpTo(usize, usize, Text, Text),
    OpRename(
        usize,
        usize,
        BTreeSet<usize>,
        std::collections::BTreeMap<usize, Text>,
    ),
    OpDelete(usize),
    FlushDel(usize),
    PowerK(BTreeSet<usize>),
    WClose(usize),
    WRecv(usize),
    DeliverUp(bool),
    Observe(usize),
    Flush(usize),
    Check,
    Rename,
    DirSync(bool),
    SaveFail,
    DraftSync(usize),
    SwitchReq,
    SwitchFin,
    ExtWriteD(usize, Text, bool),
    Crash,
    WindowCrash,
    Power(bool, bool),
    Launch,
}
/// Quint: step (explicit nondeterministic choice, with the original finite domains).
fn step(x: &State, a: Action) -> Option<State> {
    let n = x.s.pages.len();
    let in_domain = match &a {
        Action::WEdit(p, v) | Action::WResolve(p, v) => *p < n && real(*v),
        Action::WOp(p, ds, dd) => *p < n && real(*ds) && real(*dd),
        Action::ExtWriteD(p, v, _) => *p < n && (real(*v) || *v == ABSENT),
        Action::WOpTo(p, q, ds, dd) => *p < n && *q < n && real(*ds) && real(*dd),
        Action::OpRename(p, q, refs, rt) => {
            *p < n
                && *q < n
                && refs
                    .iter()
                    .all(|r| *r < n && rt.get(r).is_some_and(|t| real(*t)))
        }
        Action::PowerK(keep) => keep.iter().all(|p| *p < n),
        Action::OpDelete(p)
        | Action::FlushDel(p)
        | Action::WOpen(p)
        | Action::WSend(p)
        | Action::WDiscard(p)
        | Action::WClose(p)
        | Action::WRecv(p)
        | Action::Observe(p)
        | Action::Flush(p)
        | Action::DraftSync(p) => *p < n,
        _ => true,
    };
    if !in_domain {
        return None;
    }
    match a {
        Action::WOpen(p) => w_open(x, p),
        Action::WEdit(p, v) => w_edit(x, p, v),
        Action::WSend(p) => w_send(x, p),
        Action::WResolve(p, v) => w_resolve(x, p, v),
        Action::WDiscard(p) => w_discard(x, p),
        Action::WOp(p, a, b) => w_op(x, p, a, b),
        Action::WOpTo(p, q, a, b) => w_op_to(x, p, q, a, b),
        Action::OpRename(p, q, refs, rt) => op_rename(x, p, q, &refs, &rt),
        Action::OpDelete(p) => op_delete(x, p),
        Action::FlushDel(p) => flush_del(x, p),
        Action::PowerK(keep) => power_k(x, &keep),
        Action::WClose(p) => w_close(x, p),
        Action::WRecv(p) => w_recv(x, p),
        Action::DeliverUp(ok) => deliver_up(x, ok),
        Action::Observe(p) => observe(x, p),
        Action::Flush(p) => flush(x, p),
        Action::Check => check(x),
        Action::Rename => rename(x),
        Action::DirSync(ok) => dir_sync(x, ok),
        Action::SaveFail => save_fail(x),
        Action::DraftSync(p) => draft_sync(x, p),
        Action::SwitchReq => switch_req(x),
        Action::SwitchFin => switch_fin(x),
        Action::ExtWriteD(p, v, d) => ext_write_d(x, p, v, d),
        Action::Crash => crash(x),
        Action::WindowCrash => window_crash(x),
        Action::Power(a, b) => power(x, a, b),
        Action::Launch => launch(x),
    }
}

#[cfg(test)]
mod tests;

/// Test-only access to the existing oracle and scenario expression evaluator.
/// The host cannot reach this module in a non-test build.
#[cfg(test)]
pub(crate) mod conformance {
    use super::*;
    use serde_json::Value;

    #[derive(Clone)]
    pub(crate) struct Oracle(State);

    impl Oracle {
        pub(crate) fn model_sha() -> &'static str {
            MODEL_SHA
        }

        pub(crate) fn new(profile: &str, pages: usize) -> Self {
            Self(init(Config {
                pages,
                r1: profile == "R1" || profile == "all",
                weak: profile == "weak" || profile == "all",
                mutant: "none".into(),
            }))
        }

        pub(crate) fn state(&self) -> Value {
            serde_json::to_value(&self.0).unwrap()
        }

        pub(crate) fn next(&self, name: &str, args: &[Value]) -> Option<Self> {
            step(&self.0, tests::action(name, args)).map(Self)
        }

        pub(crate) fn eval(&self, expression: &Value) -> Value {
            tests::eval(expression, &self.0)
        }

        pub(crate) fn observed_guarantee(&self, observed: &Value) -> bool {
            let mut state: State = serde_json::from_value(observed.clone()).unwrap();
            state.config = self.0.config.clone();
            guarantee(&state)
        }

        /// Run only the guarantee instrumentation on observed states. No
        /// action rule or expected successor supplies any observed field.
        pub(crate) fn observed_commit(&self, before: &Value, after: &Value, tag: &str) -> Value {
            let mut before: State = serde_json::from_value(before.clone()).unwrap();
            before.config = self.0.config.clone();
            let after: State = serde_json::from_value(after.clone()).unwrap();
            serde_json::to_value(commit(&before, after.s, after.g, tag).unwrap()).unwrap()
        }

        pub(crate) fn eval_observed(&self, expression: &Value, observed: &Value) -> Value {
            let mut state: State = serde_json::from_value(observed.clone()).unwrap();
            state.config = self.0.config.clone();
            tests::eval(expression, &state)
        }
    }
}
