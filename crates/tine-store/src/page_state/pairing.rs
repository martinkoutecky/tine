//! Durable rename vehicles and their in-memory recovery state in the Quint oracle.
use super::order::gate_obligations;
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    test,
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "camelCase")
)]
pub(super) enum Phase {
    Armed,
    Held,
    Moved,
    Blocked,
    Restore,
    Retained,
    Settled,
    Cancelled,
    Disarmed,
    SourceDisk,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    test,
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "camelCase")
)]
pub(super) enum Decision {
    None,
    Cancelled,
    Disarmed,
    SourceDisk,
    Unreadable,
}

impl Pair {
    pub(super) fn unresolved(&self) -> bool {
        matches!(
            self.phase,
            Phase::Armed
                | Phase::Held
                | Phase::Moved
                | Phase::Blocked
                | Phase::Restore
                | Phase::Retained
        )
    }

    pub(super) fn source(&self, p: usize) -> bool {
        self.src == p && self.unresolved()
    }

    fn gated(&self) -> bool {
        matches!(
            self.phase,
            Phase::Armed | Phase::Held | Phase::Moved | Phase::Blocked
        )
    }

    pub(super) fn member(&self, p: usize) -> bool {
        self.records[p].bytes != NONE
    }

    fn original(&self, drafts: &[Draft], p: usize) -> bool {
        self.member(p) && self.records[p] == drafts[p]
    }

    pub(super) fn certified(&self, disk: &[Text], stable: &[Text], weak: bool) -> bool {
        let image = self.records[self.dst].bytes;
        disk[self.dst] == image && (stable[self.dst] == image || weak)
    }

    fn source_present(&self, disk: &[Text], stable: &[Text], weak: bool) -> bool {
        let preimage = self.records[self.src].base;
        disk[self.src] == preimage && (stable[self.src] == preimage || weak)
    }

    pub(super) fn restorable(&self, disk: &[Text], trash: &[BTreeSet<Text>]) -> bool {
        disk[self.src] == ABSENT && trash[self.src].contains(&self.records[self.src].base)
    }

    fn cancel_phase(
        &self,
        disk: &[Text],
        stable: &[Text],
        trash: &[BTreeSet<Text>],
        config: &Config,
        at_launch: bool,
    ) -> Phase {
        let witnessed = !at_launch || config.weak || config.mutant("MUseLaunchBytes");
        if self.source_present(disk, stable, config.weak)
            || (witnessed && self.certified(disk, stable, config.weak))
        {
            Phase::Cancelled
        } else if self.restorable(disk, trash) {
            Phase::Restore
        } else {
            Phase::Retained
        }
    }

    pub(super) fn reset(&mut self, config: &Config) {
        if config.mutant("MForgetChoice") {
            self.decision = Decision::None;
        }
        self.phase = match self.decision {
            Decision::None => Phase::Armed,
            Decision::Unreadable => Phase::Blocked,
            Decision::Cancelled => Phase::Cancelled,
            Decision::Disarmed => Phase::Disarmed,
            Decision::SourceDisk => Phase::SourceDisk,
        };
    }
}

pub(super) fn install_pairs(
    s: &mut Sys,
    moves: &BTreeMap<usize, usize>,
    footprint: &BTreeSet<usize>,
    vehicle: i64,
) {
    let endpoints: BTreeSet<_> = moves.keys().chain(moves.values()).copied().collect();
    let refs: BTreeSet<_> = footprint.difference(&endpoints).copied().collect();
    for (&dst, &src) in moves {
        let records = s
            .pages
            .iter()
            .enumerate()
            .map(|(p, page)| {
                if p == dst || p == src || refs.contains(&p) {
                    draft_entry(page)
                } else {
                    nodraft()
                }
            })
            .collect();
        s.pairs.push(Pair {
            vehicle,
            dst,
            src,
            records,
            phase: Phase::Armed,
            decision: Decision::None,
        });
    }
}

pub(super) fn can_draft(s: &Sys) -> bool {
    s.owners.contains(&0) && s.draft_ok
}

pub(super) fn retained(s: &Sys, p: usize) -> bool {
    s.pairs.iter().any(|pair| {
        pair.member(p)
            && s.pairs
                .iter()
                .any(|sibling| sibling.vehicle == pair.vehicle && sibling.unresolved())
    })
}

pub(super) fn wanted_draft(x: &State, p: usize) -> Draft {
    let wanted = draft_entry(&x.s.pages[p]);
    if wanted.bytes == NONE && !x.config.mutant("MDropRetained") && retained(&x.s, p) {
        x.s.drafts[p].clone()
    } else {
        wanted
    }
}

pub(super) fn awaits(s: &Sys, p: usize) -> bool {
    s.gates.iter().any(|gate| gate.open.contains(&p))
}

pub(super) fn decide_disk(x: &State, s: &mut Sys, p: usize, src: Option<usize>) {
    let shared = x
        .config
        .mutant("MSharedChoice")
        .then(|| {
            s.pairs
                .iter()
                .find(|pair| pair.dst == p && src == Some(pair.src))
                .map(|pair| pair.vehicle)
        })
        .flatten();
    for pair in &mut s.pairs {
        if ((pair.dst == p && src == Some(pair.src)) || shared == Some(pair.vehicle))
            && (pair.unresolved() || pair.decision == Decision::SourceDisk)
        {
            pair.decision = Decision::Cancelled;
        } else if pair.source(p) && pair.decision == Decision::None {
            pair.decision = Decision::SourceDisk;
        } else {
            continue;
        }
        pair.phase = pair.cancel_phase(&s.disk, &s.stable, &s.trash, &x.config, false);
    }
}

pub(super) fn resume_pair(config: &Config, s: &mut Sys, p: usize) {
    for pair in &mut s.pairs {
        if pair.dst == p && pair.certified(&s.disk, &s.stable, config.weak) {
            pair.phase = match pair.phase {
                Phase::Held => Phase::Armed,
                Phase::Moved => Phase::Settled,
                phase => phase,
            };
        }
    }
}

pub(super) fn published_pair(x: &State, s: &mut Sys, g: &mut Ghost, p: usize, bytes: Text) {
    resume_pair(&x.config, s, p);
    for pair in s.pairs.iter_mut().filter(|pair| pair.source(p)) {
        let certified = pair.certified(&s.disk, &s.stable, x.config.weak);
        let present = pair.source_present(&s.disk, &s.stable, x.config.weak);
        match (bytes, pair.phase) {
            (ABSENT, Phase::Armed) => {
                let exact = certified
                    && s.disk[p] == ABSENT
                    && (s.trash[p] == s.trash_stable[p] || x.config.weak);
                if x.config.mutant("MSkipPostcheck") && !certified {
                    g.bad.insert("settled-without-exact-receiver".into());
                }
                pair.phase = if exact || x.config.mutant("MSkipPostcheck") {
                    Phase::Settled
                } else {
                    Phase::Moved
                };
            }
            (text, Phase::Armed | Phase::Held) if text != ABSENT => {
                pair.phase = if present || certified || x.config.mutant("MDropPreimage") {
                    Phase::Disarmed
                } else {
                    Phase::Retained
                };
                pair.decision = Decision::Disarmed;
            }
            (_, Phase::Restore) if present => pair.phase = Phase::Cancelled,
            _ => {}
        }
    }
}

pub(super) fn recovery_draft(s: &Sys, p: usize) -> Draft {
    if s.pairs.iter().any(|pair| {
        ((pair.decision == Decision::Cancelled && (pair.src == p || pair.dst == p))
            || (matches!(pair.decision, Decision::SourceDisk | Decision::Disarmed)
                && pair.src == p))
            && pair.original(&s.drafts, p)
    }) {
        nodraft()
    } else {
        s.drafts[p].clone()
    }
}

pub(super) fn recover_pairs(x: &State, s: &mut Sys, g: &mut Ghost) {
    for pair in &mut s.pairs {
        if matches!(
            pair.decision,
            Decision::Cancelled | Decision::Disarmed | Decision::SourceDisk
        ) {
            pair.phase = pair.cancel_phase(&s.disk, &s.stable, &s.trash, &x.config, true);
        }
    }
    let mut vehicles: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for pair in s.pairs.iter().filter(|pair| pair.gated()) {
        vehicles.entry(pair.vehicle).or_default().push(pair);
    }
    for pairs in vehicles.values() {
        let open: BTreeSet<_> = (0..s.pages.len())
            .filter(|&p| s.pages[p].held && pairs.iter().any(|pair| pair.member(p)))
            .collect();
        if !pairs
            .iter()
            .any(|p| open.contains(&p.src) || open.contains(&p.dst))
        {
            continue;
        }
        let vers: Vec<_> = s
            .pages
            .iter()
            .enumerate()
            .map(|(p, pg)| if open.contains(&p) { pg.ver } else { NONE })
            .collect();
        let gate = Gate {
            moves: pairs.iter().map(|p| (p.dst, p.src)).collect(),
            vers,
            open,
        };
        g.obl.extend(gate_obligations(&gate));
        if !x.config.mutant("MForgetGates") {
            s.gates.push(gate);
        }
    }
}

pub(super) fn pair_rewrite(x: &State, src: usize, ok: bool) -> Option<State> {
    if !x.s.alive || !x.s.owners.contains(&0) {
        return None;
    }
    let target =
        x.s.pairs
            .iter()
            .rfind(|pair| pair.src == src && !pair.unresolved())?;
    let survives = |pair: &Pair| {
        pair.vehicle != target.vehicle || (pair.src != src && !x.config.mutant("MDropSibling"))
    };
    let mut s = x.s.clone();
    if (ok || x.config.mutant("MRewriteOnFailure")) && s.draft_ok {
        for p in 0..s.pages.len() {
            let shared = s
                .pairs
                .iter()
                .filter(|pair| survives(pair))
                .any(|pair| pair.vehicle == target.vehicle && pair.member(p));
            if target.original(&s.drafts, p)
                && (p == src || p == target.dst || !s.pages[p].risk)
                && (!shared || x.config.mutant("MDropShared"))
            {
                s.drafts[p] = nodraft();
            }
        }
        if !x.config.mutant("MKeepSettled") {
            s.pairs.retain(survives);
        }
    }
    commit(x, s, x.g.clone(), "pairRewrite")
}

pub(super) fn disjoint_pairs(s: &Sys) -> bool {
    let mut endpoints = BTreeSet::new();
    s.pairs
        .iter()
        .all(|p| endpoints.insert((p.vehicle, p.src)) && endpoints.insert((p.vehicle, p.dst)))
}

pub(super) fn retained_records(s: &Sys) -> bool {
    s.pairs.iter().filter(|p| p.unresolved()).all(|pair| {
        s.drafts
            .iter()
            .enumerate()
            .all(|(p, draft)| !pair.member(p) || draft.bytes != NONE)
    })
}

pub(super) fn restore_source(x: &State, src: usize) -> Option<State> {
    let state = &x.s;
    let pg = &state.pages[src];
    if !state.alive
        || !can_draft(state)
        || locked(x, src)
        || state.claimed.contains(&src)
        || (pg.held && !clean(pg))
    {
        return None;
    }
    let index = state.pairs.iter().rposition(|pair| {
        pair.src == src
            && matches!(pair.phase, Phase::Restore | Phase::Retained)
            && matches!(pair.decision, Decision::Cancelled | Decision::SourceDisk)
            && pair.restorable(&state.disk, &state.trash)
    })?;
    let mut s = state.clone();
    let mut g = x.g.clone();
    g.vc = next(g.vc);
    g.op_read[src].insert(ABSENT);
    s.pairs[index].phase = Phase::Restore;
    s.pages[src] = Page {
        held: true,
        buf: s.pairs[index].records[src].base,
        ver: g.vc,
        risk: true,
        typed: true,
        ..nopage()
    };
    commit(x, s, g, "restoreSource")
}

pub(super) fn other_owner(x: &State, claim: bool) -> Option<State> {
    let mut s = x.s.clone();
    if claim && s.owners.is_empty() {
        s.owners.insert(1);
    } else if claim || !s.owners.remove(&1) {
        return None;
    }
    commit(x, s, x.g.clone(), "owner")
}

pub(super) fn draft_store(x: &State, ok: bool) -> Option<State> {
    if !x.s.alive || !x.s.owners.contains(&0) || x.s.draft_ok == ok {
        return None;
    }
    let mut s = x.s.clone();
    s.draft_ok = ok;
    commit(x, s, x.g.clone(), "draftStore")
}

pub(super) fn unreadable_decision(x: &State, dst: usize) -> Option<State> {
    let mut s = x.s.clone();
    if !s.alive
        || !s
            .pairs
            .iter()
            .any(|p| p.dst == dst && p.phase == Phase::Armed)
    {
        return None;
    }
    for pair in s
        .pairs
        .iter_mut()
        .filter(|pair| pair.dst == dst && pair.phase == Phase::Armed)
    {
        pair.phase = Phase::Blocked;
        pair.decision = Decision::Unreadable;
    }
    commit(x, s, x.g.clone(), "unreadableDecision")
}
