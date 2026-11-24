# 0082. A rename's pairing is recorded in its operation draft

- **Status:** Accepted
- **Date:** 2026-11-24

## Context

A page rename or merge is one page-host operation over a receiver (the new
page), its referrers and a source (the old page, removed to Tine's trash).
While Tine runs, `page_host/order.rs` orders the writes (receiver, then
referrers, then the source's removal), and a Discard of the unwitnessed
receiver reverts the source. That pairing lived only in memory. At launch the
operation draft was exploded into independent page drafts, so a recovered
source deletion was ungated: after a crash, Use Disk on the recovered
receiver (or a sync-delivered conflict on it) let the source be trashed with
no page holding its text (commit `68ed8578a` characterizes both cases).

The draft store's format (`drafts-v2/<graph id>`, the `TINEDRF2` envelope,
version byte, postcard body and SHA-256 checksum, ADR 0061 and
`page_host/drafts.rs`) is persisted app data that older Tine versions also
read, so any change has to stay safe for them.

## Decision

We extend the draft body, keeping the envelope, its version byte and the file
names:

- A legacy body is a nonempty postcard record vector and recovers exactly as
  before.
- An extended body starts with the escape byte `0x00` (a legacy body never
  does: its vector is nonempty). Tag `0` is a **paired operation draft**
  (`op-<uuid>.draft`): the ordered records, then a postcard `Vec<(u32, u32)>`
  of zero-based receiver/source record indexes. The filename UUID is the
  request id. Pairs must be in bounds and disjoint, each source a deletion
  with a known present base and each receiver a present proposal.
- Tag `1` is a **decision record**
  (`p-<page-hash>-<nonce>-<op-uuid>-<member>.draft`): 16 raw UUID bytes, the
  receiver's member index (`u32`), a new write sequence (`u64`) and a choice
  byte. `0` is take disk (Use Disk), with no payload. `1` (reviewed text) and
  `2` (superseding input) carry a page version, a `Base` and a `Text`; they
  are decoded and validated now and acted on by a later change; until then
  an operation with one blocks its source removal.
- An older reader decodes an extended body as an empty vector, rejects it and
  quarantines the file, so it never replays a paired source as a standalone
  deletion. A malformed paired vehicle is quarantined intact. An unreadable or
  mismatched decision naming a readable paired operation stays in place and
  blocks that operation's removals.

The page host's `Order` owns each paired operation's obligations, removal
permission and settlement, adopted at the draft's durable terminal and
reinstalled by `Host::launch` from the scan before any recovered work. A
source is removed only after its receiver's disk bytes equal the certified
receiver image immediately before the trash step and again after the
source's durability steps. Use Disk on an unwitnessed receiver writes its
decision durably before it takes effect. The paired draft is immutable and
retires whole once every pair is settled or cancelled: superseded page
drafts, then the operation draft, then its decisions.

Unit cost (I-25): five bytes per singleton rename (escape, tag, pair count,
two small indexes), no added file, fsync, launch read or listing on the clean
path, and two receiver reads per removed source. The 1- and 60-block fixtures
write 305 and 4,142 bytes (300 and 4,137 before), the 60-block source 1,900. A
Use Disk decision is one 62-byte private file and two publication syncs.

## Consequences

- A cancelled rename no longer leaves a page only in trash after a crash, and
  a durable cancellation is never replayed as a removal.
- Downgrading Tine with a paired draft pending quarantines it: the recovered
  rename's text stays in the quarantine directory and on disk at the source,
  and the rename has to be retried.
- An operation over a page of an unretired paired draft waits for it
  (no chaining), slightly longer than for the in-memory order alone.
- Not yet covered (follow-up): restoring a source that was already moved when
  its receiver diverged (the draft keeps the text meanwhile), reviewed-text
  and superseding decisions, and the recovery panel for an occupied source.
