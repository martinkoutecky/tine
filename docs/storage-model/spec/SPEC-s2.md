# og storage — specification s2 (2026-10-08, revised after final review)

> **Publication note (2026-10-14).** This is the working specification as the project used it, published
> with three kinds of edits only: this note, removed host names, and private directory prefixes stripped
> from file references. Names such as `s3-lean7.md`, `s3lean6`, `logs/…`, `review/…`, snapshot or patch
> files refer to internal working records (prover/verifier notes, lane directories, run logs) that are not
> published; [`../README.md`](../README.md) summarises what they established. The published models and
> scenarios are in [`../quint/`](../quint/) and the Lean proofs in [`../lean/`](../lean/). Identify a model
> version by its SHA-256 prefix; the README maps every prefix to its published file.

`storage-s2.qnt` **is** the specification (sha256 prefix `1c3194293856a116`). This file explains it for
implementers and reviewers. Where they differ, the model wins. It replaces `storage.qnt`/`SPEC-storage.md`
(draft 25, round-2 model with an operation record), which are kept as history. The previous version of this
file described model `846da970`; §6 lists what the final reviews changed.

Design decisions are Martin's of 2026-10-07 (OS1/OS2) and 2026-10-08.

**Erratum s2.1 (2026-10-09):** `observe` must apply every read that changes the page, also one equal to the
last read (`storage-s2x.qnt`; why, and the proof rebuild, in `SPEC-s3.md` §1a). s3 (rename/delete) extends
s2.1.

## 1. Shape

- **The backend owns every page.** It holds each open page's buffer and does all file I/O. A page is never
  touched by two backend steps at once: a save holds the page from start to answer, and the window's requests
  for that page wait.
- **The window is an editor client.** It sends its text together with the backend version it was typed on, and
  shows what the backend pushes. One request per page is on its way at a time (debounced in the code). One
  editing window per page (§4.10).
- **A move between pages is two ordinary edits, with no operation record.** Applying a move first writes the
  receiving page's new text to the draft store; if that write fails, or either page's version is stale, the
  move is refused whole and nothing changes. So an interrupted move leaves a duplicate, never a loss (on a
  filesystem that keeps an fsynced rename). An interrupted move is neither completed nor undone (accepted, as
  in original Logseq).
- **The draft store mirrors the backend.** A page has a draft when it is *at risk*. The mirror is eventual: it
  holds exactly at every graph-switch barrier, not between a change of `risk` and the next draft write. Drafts
  live in app data, one per page, newest state written.
- **Scope:** editing text at a fixed path, and moving text between paths. Page rename and page delete are
  not modelled; they need their own guarded design before their code paths are touched (§7).

## 2. Page state

| Field | Meaning |
|---|---|
| `buf`, `base` | buffer bytes; the disk bytes the buffer is based on. `UNKNOWN` base: text typed on a version the backend had moved past |
| `ver` | buffer version; changes on every buffer change |
| `typed` | the buffer holds input Tine has not itself published |
| `risk` | has (or is getting) a draft |
| `conflict` | disk changed under unpublished input |
| `obs` | the disk bytes last read |

Clean = not typed, `buf == base`, not at risk.

## 3. Rules

**Reading the path**, applied by a watcher/rescan read, a failed guard check, an open:
1. bytes equal the base: nothing changes (a conflict clears);
2. a clean page loads them (new version);
3. bytes equal the buffer: the base advances, nothing is published. The input is still Tine's to save, and a
   later external change raises a conflict. (An external change wins only over text Tine itself published by a
   guarded rename.) This also applies to an `UNKNOWN` base: a read equal to the buffer clears the conflict and
   identical bytes may then be saved.
4. otherwise: conflict, at risk.

A failed read changes nothing; a failed open or discard read is refused visibly.

**Saving (guarded publish):** write and fsync a temp file; guard check (the target must hold exactly the base,
else the read is applied as an observation: Mismatch); rename; directory fsync. Outcomes:
- *Published* (§4.7 defines per platform when the rename counts as published): base := bytes, not typed, not
  at risk.
- *Uncertain* (the directory fsync returned an error): stays at risk.
- *Failed* (any error before the rename): stays at risk.

Only a Published reply ends risk. A page with a conflict is not saved.

**Window requests, in order, per page:**
- *submit* on the version it was typed on: an edit. On any other version: the text becomes a conflict with an
  `UNKNOWN` base, at risk, not saved until the user resolves or discards it (or a read shows the buffer's
  exact bytes, rule 3).
- *resolve* (keep mine / choose): the user approves bytes against the disk bytes they were shown.
- *discard*: the buffer takes the disk bytes. **A discard claims no durability:** Tine cannot tell whether the
  bytes it reads are durable (its own Uncertain or interrupted save, or another program's write that a power
  cut may still revert). So if the page is at risk and its buffer or its draft holds exactly the bytes read,
  it stays at risk, and Tine saves those bytes itself before the draft goes. A discard that shows a live
  promise's bytes leaves the promise as it is; any other promise ends (the user chose the disk's bytes).
  Discard consumes the window's unsent input for that page **even when the read fails and the discard is
  refused**: the user asked to drop that input (the window must say the discard failed; §4.11).
- *close*: a clean page is retired; a dirty one stays.
- *move*: refused whole if either page's version is stale or the receiver's draft write fails; otherwise
  receiver's draft first, then both pages take their texts as submits; the receiver stays at risk until saved.

**Every answer** carries the request's identity, window generation, the resulting version, and `took`: whether
the page's buffer is now the text the request carried. Text typed while the request was on its way moves to the
answer's version only if `took` (an applied submit or move); after a discard or a refused move it keeps its old
version, so it becomes a conflict if sent.

**Pushes** replace the window's text only when nothing is typed or on its way.

**Graph switch / window close:** every dirty page is put at risk (drafted). Prefer saving a dirty,
non-conflicted page over drafting it (§4.9). The switch completes only when the window has sent everything,
the backend has taken it, no save runs, every page is clean or at risk, and the draft store mirrors the pages
exactly (including queued draft *removals*).

**Launch / return to the graph:** every draft becomes its page's buffer, at risk, with a fresh version, before
anything else touches the page. Text typed on an older version is restored with its `UNKNOWN` base. The
guarantee needs no directory sync at launch (the previous version required one); a best-effort one is still
done (§4.9), because it narrows a loss of unacknowledged text (§5).

**External writes** may be readable before they are durable: a power cut may revert another program's
write to the previous stable bytes. (The model's `extWriteD(p, v, dur)`; this holds in every profile.)

## 4. Implementation obligations (assumed by the model; to be checked by conformance tests)

1. A page's reads and the application of their result are serialized; an older read never applies after a
   newer one.
2. Draft writes and removals for a page are serialized and carry the version they write; one older than the
   store holds is never applied.
3. A move holds both pages (deterministic lock order) from its version check and draft write to their update.
4. The backend takes a window request (it is in the request queue) before the window counts it as sent.
5. Every answer carries its request's identity and window generation; an answer for an earlier window or
   another request is dropped. Every answer carries `took`.
6. An orderly switch drains the window, the backend and every in-flight draft write before it closes. A
   forced mobile termination is a crash, not a switch.
7. **Publication contract, per platform.** A rename counts as *Published* when:
   - Linux, macOS, iOS: temp `fsync` (on Apple: `F_FULLFSYNC`), atomic `rename`, then `fsync` of the
     directory, all succeeded;
   - Windows: `FlushFileBuffers` on the temp file, then `MoveFileEx(REPLACE_EXISTING | WRITE_THROUGH)`
     succeeded; no directory sync is attempted;
   - Android (direct filesystem access): as Linux; a directory `fsync` that returns `EINVAL`/unsupported
     is treated as unsupported, not as an error.

   Where directory sync is unsupported, the platform is in the **weak** profile (R3 accepted: an
   acknowledged save may revert on power cut). "Uncertain" means an actual error, never "unsupported";
   otherwise every save would end Uncertain and be retried forever. An error class a filesystem returns on
   every directory sync (not just `EINVAL`) is classified by that behaviour as unsupported, i.e. weak.

   **Scope of the claim (Martin, 2026-10-08, DD2/DD3).** The guarantee and its proofs are claimed for the
   strong profile: Linux (and the other recipes above) on a local filesystem with atomic rename and directory
   sync. Every other platform or store runs the same code and is written **best-effort, outside the
   guarantee, never refused**: Tine refuses nothing in an ordinary setup that Beta writes today. Implementation
   obligation: an "ordinary setups" conformance matrix (Linux ext4/btrfs, Windows NTFS, Android all-files
   access, macOS APFS, an SMB/NFS share, a FAT/exFAT USB stick, a Syncthing or Dropbox folder) in which every
   setup that saves on Beta still saves; setups that cannot be run are listed as unverified.
   These recipes name the API sequence; they do not certify every backing store. A store qualifies only when it
   actually gives atomic replace with old-or-new outcomes (classify network and removable volumes before
   writing). The model assumes the draft store is strong even when the graph is weak. Where app data itself has
   no directory sync (e.g. Windows), that assumption rests on the platform's metadata journaling for
   write-through renames; it is an open point to confirm per platform (lower confidence, not a shipping blocker).
   A store whose replace is not atomic (delete+create, in-place write; e.g. SAF document providers, some
   network or removable volumes) is outside this model: it is **written best-effort, outside the guarantee,
   with no refusal and no special state**, as Beta does today. Its saves are reported as Published under the
   weak profile for progress purposes only; the guarantee makes no claim about them.
8. **The draft store is itself durable and checked:** each draft write is temp + fsync + rename + directory
   fsync (a draft counts as written only once its directory entry is durable; a checksum detects damage but
   does not preserve a copy), carries a checksum, and a draft removal is durable before the page is reported clean to a switch. A draft that fails
   to decode at launch is kept aside and shown as unrecoverable, never deleted, and never refuses the graph.
   The store is one file per page, so it has no record-count bound; it never refuses a draft for size. (Beta's
   current `drafts.rs` bounds, 64 records and 8 MiB with writes past them refused, do not carry over.) If a
   draft write itself fails, the page stays at risk and the UI shows it (§4.11); nothing else is refused.
9. **Progress (not in the model; the model is safety-only).** A dirty page is saved, or drafted if the save
   fails, within a stated bound after its last change (target ≤ 1 s). A conflict's draft is written before
   the conflict is shown. A save that keeps ending Failed **or Uncertain** retries with backoff (an Uncertain
   page is dirty, so without backoff it would rename identical bytes in a loop), surfaces an actionable
   error, and never blocks a switch forever (the page is drafted instead). Best effort, unmodelled: at launch,
   fsync the graph's directories before opening pages; it only makes earlier renames durable sooner. At a switch, a dirty non-conflicted page is saved rather than only
   drafted when the save can succeed.
10. **One editing window per page.** A second window on the same page is read-only (multiwindow v1);
    otherwise every interleaved edit becomes a stale submit and a conflict.
11. **UI states.** "Accepted", "recoverable" (drafted) and "saved" (Published) are distinct. A persistent
    indicator lists pages at risk or in conflict, surfaced again at launch. A refused discard says so.

## 5. The guarantee

- **A. No loss.** Tine promises, per page, the newest text it has acknowledged: by a Published save (dated at
  its guarded rename), a durable draft, an applied move (the receiver's new text), or any other non-user step
  that shows a page as saved. After any crash, power cut or graph switch, each promised text, or a newer text
  of that page that Tine wrote, is at its path or in the draft store. A promise ends only when: another program
  writes the path after Tine's own guarded rename put the promised bytes there; the user discards and the bytes
  shown differ; a weak-filesystem power cut.
- **B. Unsaved is never replaced.** Outside user actions, crash and power cut, a backend step that replaces or
  drops a buffer finds its text acknowledged, or holding no unpublished input and equal to the newest disk
  read. Closing the graph also accepts a buffer whose text is in the draft store. A submit typed on an older
  version may replace only a buffer that is safe, whose bytes the user was shown, or that holds no user input.
- **B′. What the window sent is taken.** The window's text is replaced only once the backend has it; a
  submitted text becomes the page's buffer; until then it is in the request queue (a window reload included).
- **C. No blind overwrite.** A Tine write never replaces bytes other than those its guard check found, except
  in race R1.
- **G. No overwrite of unseen changes.** A Tine save never replaces file bytes the user was never shown, unless
  Tine wrote them itself, across crashes too, except in race R1 (G checks the bytes the guard found). Bytes are
  compared as bytes.

**Accepted (not losses):**
- R1: another program writing between guard check and rename. Its text is silently overwritten; no
  in-process lock can close this race.
- R3: a weak filesystem reverting an acknowledged save on power cut (including moved text).
- An external write after Tine's published save (it wins).
- Input that was never acknowledged: unsent input lost by a window reload, and text the backend accepted but
  had not yet saved or drafted when the whole app crashed. §4.9 bounds this window; the model does not.
- Text the user saw but Tine never acknowledged as durable, reverted by a power cut: Tine's own Uncertain
  save chosen by a later discard after the user typed on; a rename interrupted before its directory sync,
  loaded clean at relaunch (§4.9's launch sync narrows this); another program's non-durable write chosen by a
  discard of a page that was not at risk. The previous model's `unc` and launch sync covered the first two.
- A duplicate left by an interrupted move.
- A refused discard consuming the unsent input it was asked to drop.

## 6. Changes after the final reviews (2026-10-08 evening)

Astra (`gpt-6-astra`) and Fable reviewed model `846da970`
(`s2-final-review-{astra,fable}.md`). Model changes:

| Change | Kind | Scenario / mutant |
|---|---|---|
| External writes may be non-durable (`extWriteD`); a discard showing bytes its at-risk buffer or draft holds keeps the page at risk, so Tine resaves them | **real defect** (Astra 1: an equal-byte discard over another program's unflushed write cleared the draft; a power cut lost the text) | mMDE, qDiscardOverVolatileRestored, qDiscardOverVolatileResaves, qDiscardOverPublishedNoResave, MDE |
| The previous discard promotion, the `unc`/`cold` flags, the launch directory sync and `launchWith(ok)` are removed; the new discard rule covers every acknowledged text they protected | simplification (and removes Fable F3's open-every-page churn). Two losses of *unacknowledged* text they also prevented are now accepted (§5; Fable re-check) | qSavedWatermark; mutants LD, ML, MLY went with their mechanisms |
| A stale move is refused whole | **real defect** (Fable F1: a half-applied stale move, then a discard, lost the moved text) | qMoveStale |

Doc-only changes from the reviews: §4.7–4.11 (publication contract per platform, durable draft store,
progress, one editor per page, UI states), the accepted list (full-app crash before acknowledgement, R1's
silent overwrite, refused discard), G's R1 exception, rule 3 for `UNKNOWN`, the eventual draft mirror, and the
scope note on rename/delete.

No mutant or positive-control coverage was lost: all 19 mutants are caught. (B's permitted stale replacement
was deliberately broadened earlier on 10/08, with a positive control, `review/bcontrol.inc`, that still flags
typed unseen bytes.)

## 7. Verification status (model `1c319429`)

| Evidence | Result |
|---|---|
| Directed scenarios | 88/88 pass in each race profile (base, R1, weak, all) |
| Mutants (`sweep.py`) | 19/19 caught: MC MC1 MQ MLS MLO MCL MX MST MWA MOD MIS MDQ MRB MRE MAK MDO MBD MDA MDE |
| Review controls (`review/`) | 96 scenarios, exactly the 7 expected failures |
| Unbounded proof, 2 pages × 13 texts | A (all profiles), B, B′(1), B′(2), `accepted`, C, G: 14 Apalache-checked inductive invariants on abstractions + written simulation arguments (round 7, `s2-inductive7.md`; A's custody argument repaired, the rest transferred from rounds 5–6). Independent verifier `s2-inductive7-verify.md`: reran the checks, 0 findings |
| Cutoff | a violation in any graph (any pages, any texts) implies one at 2 pages × 13 texts; within bounds (V,Q,E) it maps to (2V, max(Q,1), E); 9 texts suffice within (4,1,1). Notes `s2-cutoff6.md` (stale-move refusal and restoration-cleanup steps repaired), verifier `s2-cutoff6-verify.md`: 0 findings |
| Lean 4 (no abstraction, no cutoff) | model transcribed for any P pages; replay 2,024 outcomes = Quint, 19/19 mutants. Kernel-proved, standard axioms only for every P, texts 1..T, every profile: C (`C_reachable`, `lean1`), G (`G_positiveTexts`), B′(2) (`Bprime2_reachable`, `lean3`), B (`B_reachable`, `lean4`), B′(1) (`Bprime1_reachable`) and `accepted` (`accepted_reachable`, `lean5`), A (`noLoss_positiveTexts`) and the literal whole `guarantee` (`guarantee_positiveTexts`, `lean6`; 540 theorems audited). Notes `s2-lean{1..6}.md`, verifiers `s2-lean{1..6}-verify.md`: 0 blockers (round 6: 0 findings). Differential test (`diff1`, `s2-diff1.md`, 2026-10-09): 24,000 random Quint traces (2.4M transitions, P=2, texts 1..3, 4 profiles) replayed through Lean's `step` with every state and ghost field compared after every transition: 0 differences; 9,984 enabledness probes at sampled reachable states: 0 differences; mutant controls agree, and three minimal different-mutant controls show the comparison detects single-field differences a verdict-only check would miss. Finite evidence, not a proof of equivalence |
| Bounded TLC, 2 pages × 3 texts, (4,1,1) | run `s2j` on two other machines, in progress at the time of writing (not part of the claim) |

Together: **the guarantee holds for every graph size, all versions/queue lengths/external writes (durable or
not), in all four race profiles**, under the model's abstractions. The whole guarantee is also kernel-checked
directly on the Lean model (trusting only the replay-tested transcription), which supersedes the
handwritten parts below as the primary evidence; they remain as corroboration. The
refinement from each abstraction to the model is checked by hand (prover and independent verifier), and the
cutoff reduction is a handwritten proof exercised by tests (604 executions, 280 sampled traces; verifier: 206 checks, 120 traces). Environmental assumptions: atomic replace with
old/new-only power outcomes (§4.7; non-atomic stores are outside the claim), a durable draft store (§4.8), prescribed initial states, opaque
whole-page texts compared by equality. Not covered: progress (§4.9), page rename/delete, torn or in-place
writes, document-provider semantics, and conformance of the og code to this model.
