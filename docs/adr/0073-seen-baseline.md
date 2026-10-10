# 0073. "Changed since you last looked": a per-page seen baseline in app data

- **Status:** Proposed
- **Date:** 2026-10-10
- **Authority:** vision decision 9a, Martin 2026-10-04

## Context

Pages change while the user is not looking: an agent, a script, Syncthing
or another device edits the Markdown file. When the user comes back, Tine shows
the page with no sign of what moved. Vision decision 9a asks for an opt-in
answer to "what changed since I last looked at this page?", with three
constraints:

- the graph folder stays exactly as Logseq would leave it, so nothing goes into
  the Markdown files or next to them;
- a page that the user never asked to track renders exactly as today;
- the cost of an edit scales with the edit (D-10), so per-keystroke work stays
  O(1) and nothing is written per edit.

Comparing a page with its last-seen text needs a remembered "last seen" state.
The options on the table were:

- a copy of the page text;
- a git-like history;
- block identities;
- a set of content hashes.

A text copy doubles storage, and history is a storage mode (D-17). Block
identities are not stable across an external edit, because ids are not in the
file unless a reference forces them. A set of per-block content hashes is small,
needs no ids, and makes "moved but unchanged" fall out naturally.

## Decision

**The baseline.** For each tracked page, Tine keeps one set of 64-bit hashes,
one per block, of the block's own content.

- **What counts.** A block's text, including its property lines. An agent
  setting `status:: done` is a change.
- **What does not count.**
  - Its children. A changed child marks the child, not its parents.
  - Its position. A moved, unchanged block is not a change.
  - Its fold state. `collapsed::` is removed first, so folding in Tine is not a
    change.
  - The trailing spaces that a save trims.
- **Where the rule lives.** `blockContentKey` (`src/document/edits/properties.ts`)
  defines what is hashed, and `seenBlockHash` (`src/seen/hash.ts`) is the one
  hash: cyrb53's two 32-bit lanes, kept at 64 bits. The backend stores the
  hashes as opaque values and never computes one, so there is no Rust twin.

**Where it is stored.**

- One file per tracked page, in app data and never under the graph root:
  `<app data>/seen/<graph-id>/<sha256(page identity key)>.bin`. `<graph-id>` is
  the graph's session id stem, the same per-graph key as drafts and the session.
- **Format:** the magic `TINESEEN`, a `u32` LE version (1), a `u32` LE count,
  then that many sorted, unique `u64` LE hashes. The count is at most 2^20.
- **Writes.**
  - A file is written only by **Mark page seen**, through
    `device_io::atomic_write` (temp + fsync + rename + directory sync). This is
    the one approved writer site, `src-tauri/src/seen_baseline.rs`.
  - It is removed only by **Forget seen state**, or when Tine renames, merges or
    deletes the page.
  - No edit, page open, page close or quit writes it.
- The format is entered in `PERSISTED_FORMATS` (`seen-baseline`) and in
  `docs/og-persisted-formats.md`.

**What the user sees** (routed single page only).

- **On open.** The page's baseline is read once per graph session. With no
  baseline, nothing changes: no header, no row class, no tracker.
- **With a baseline.**
  - Every block whose hash is not in the set gets a 3 px bar in the left margin.
    Its colour is `--seen-changed-color`, derived from `--accent`, so it follows
    both themes.
  - A changed page-properties header gets the same bar as an inset.
  - While at least one block is changed, a header line reads "N changes since
    you last looked" and has a **Mark seen** button.
  - Deleted blocks are not shown and not counted. The hash set cannot tell
    "deleted" from "edited", because an edit also removes the old hash.
- **How a page starts being tracked.** **Mark page seen** is in the command
  palette and in the page actions menu (⋯). **Forget seen state** sits beside it
  while the page is tracked.
- **Other surfaces show nothing.** The journal feed, the sidebar, zoomed blocks,
  references and embeds read no seen context (`seenRowClasses` returns `{}`
  there). A journal opened as a routed page is tracked like any page.

**Rename and delete policy.** Rename drops the baseline; it is never carried.

- A Tine rename or merge removes the old name's record, and the new name starts
  untracked. A Tine delete removes the record.
- An external rename leaves the old record orphaned (a few hundred bytes). A
  later page with the old name would inherit it, and would at worst show a few
  blocks as changed.
- The one excluded outcome holds: the whole page is never highlighted because of
  a rename.

**Failure behaviour.**

| Situation | Outcome | In-scope scenario |
|---|---|---|
| File missing | no baseline: the page renders untracked | — (normal) |
| File torn, foreign magic or version, length not matching its count, over the bound, unreadable | no baseline; the file stays and the next Mark replaces it; no dialog | crash or power loss mid-write on a non-atomic filesystem, disk error, a sync client delivering another build's app data |
| The load IPC fails | no baseline (logged only) | disk error, backend restart |
| Mark with more than 2^20 hashes | **refused**: the old record stays, and an error toast says the page was not marked | malformed imported Markdown producing a page with millions of blocks; the bound keeps every record readable by its own reader |
| Mark or forget payload with a hash that is not 16 hex digits, or an empty page key | **refused** as an IPC decode failure | web content and plugins are untrusted input across the IPC boundary (D-2b) |
| Mark fails (disk full, permission) | error toast; the old record and the old highlight stay | disk error |
| Mark on a Guide page or in a published export | not offered | — (no app data there) |

A missing or corrupt baseline is never an error dialog, and never a refusal to
open the page.

**Cost.**

- **Page open:** one file read, plus one hash per block.
- **Keystroke:** the edited block is rehashed, which is O(edit), and the count
  moves by at most one.
- **Structural edit:** one O(page) walk of the outline's ids, with no rehash of
  unchanged blocks.
- **Measured** on a copy of the anonymized graph's largest page (659 blocks,
  56,860 characters, Node 22, median of 21 runs):
  - hashing every block: 1.59 ms;
  - creating the tracker: 2.85 ms;
  - one edit with the count update: 0.27 ms;
  - record size: 5,080 B.

Unit cost: 24 B for a 1-block page and 496 B for a 60-block page (16 + 8 ×
distinct blocks; measured by the `seen_baseline` unit-cost test). One file is
written per Mark page seen. Each edit writes 0 bytes and 0 files, and 0 bytes
go over any transport, because the record stays in app data and is never
synced.

## Consequences

- A tracked page shows exactly which blocks differ from what the user last
  marked, at almost no cost, and the graph gains nothing.
- Two blocks with identical text count as one hash. If one was edited to match
  the other, it shows as unchanged, which is accurate.
- A block whose old text reappears elsewhere also shows as unchanged. That is
  acceptable for a "where should I look" signal.
- Deletions are invisible. A future "N blocks removed" line would need block
  identities or positions in the record, which means a new format version and a
  new decision.
- Highlights are per device. Marking a page seen on one machine does not clear
  it on another, because the record is in app data. Syncing seen state would be
  a new sync record and needs its own decision.
- An external rename leaves a small orphan record. A future sweep could prune
  records whose page key no longer resolves. Nothing depends on that.
- The hash is a frontend definition. Changing `blockContentKey` or the hash
  makes every existing baseline show the whole page as changed once. A change
  to either needs a format version bump that ignores old records.
