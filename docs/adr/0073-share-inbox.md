# 0073 — Share inbox for native capture

Status: Accepted (Martin 2026-10-10 (Rule 8 new persisted format: share-inbox); native-integrations batch, GH #608).

## Context
The iOS Share Extension, the "Add to Tine journal" App Intent, the Android
`ACTION_SEND` share target and the Android Quick Settings tile all hand Tine
text, a link or images while the app may not be running. OG (Logseq)
behaviour, approved by Martin: a shared item becomes a new block at the bottom
of today's journal (`frontend/mobile/intent.cljs`, `frontend/handler/quick_capture.cljs`).

Native code must never write graph files: graph writes go through the one
audited save path (temp + fsync + rename + base-revision guard + lock), and the
iOS extension runs in a separate process that cannot take the app's lock. The
item therefore has to wait somewhere durable until the app can write it.

No existing persisted format expresses this:
- the graph (pages, assets) is exactly what native code must not write;
- the draft store (ADR 0061) holds unsaved editor text of one page, per graph,
  inside the app; it has no resources and is not writable by a second process;
- device settings JSON is a single document rewritten by the app; a second
  writer (the extension) would race it, and binary resources do not belong in it;
- the launch checkpoint and Concord ledger are disposable caches; an inbox item
  is user data until ingested.

## Decision
One app-owned **share inbox** directory, never under a graph root:
iOS `<App Group group.page.tine.Tine>/share-inbox/` (falling back to the app's
Application Support when the App Group is not provisioned), Android
`<filesDir>/share-inbox/`. Desktop has none.

Layout, one directory per item:
- `<id>/item.json`: `{version: 1, source: "android"|"ios", created, text?, title?, url?, resources: [{file, name, type}]}`
  plus the resource files. `source` selects the producer's OG shaping path
  (src/shareShape.ts). Producers write `.tmp-<id>/`, sync every file and the
  directory, publish it with one directory rename and sync the inbox before
  they report success, so a partial item is never visible and a confirmed
  one survives power loss (Android `ShareIntake.kt` `InboxWriter`, iOS
  `ShareInbox.swift`, `F_FULLFSYNC`).
- `<id>/prepared.json`: `{graph, day, markdown, assets, armed, written}`, written by
  the app through `device_io::atomic_write` (src-tauri/src/share_inbox.rs)
  before each step that touches the graph:
  - `graph`: the canonical root of the graph the item is bound to;
  - `day`: the journal day the item lands in, frozen when it is bound
    (`{date}` is this day);
  - `markdown`, `assets`: the shaped block and the asset files imported for
    it, recorded before any append (null/empty until then);
  - `armed`: `{before}`, recorded inside the admitted read the append uses,
    immediately before the insert: the day file's content revision (null
    when absent). Diagnostic only: recovery never compares content;
  - `written`: set as soon as the append's flush returned ok, before the
    commit.
- `.trash-<id>/`: a committed item between its rename and its removal.
- `.committed-<id>`: an empty tombstone the commit writes durably before the
  item leaves; a listing removes it after 30 days. The Android producer
  reads it so a redelivered share occurrence is not published again.
- `.receiving-<id>` (Android): written durably by the producer before it
  copies a share, holding a short summary (the first 60 characters of the
  text, or "N images"); deleted once the item is published and its inbox
  sync succeeded, or the share is refused with a message. At the next start
  a leftover marker whose item or tombstone exists is deleted only after an
  inbox sync succeeds; one with neither (the process died mid-copy, or an
  unsynced rename was lost) shows "A share to Tine was interrupted and
  wasn't saved: <summary>. Please share it again." and is deleted only
  after that notice is shown, so a crash in between repeats the notice. An
  unreadable marker reports "a shared item" without affecting the others.
  The app's listing ignores it. Creating the inbox, or finding it existing,
  always syncs its parent directory before the first marker is written.
- `.rejected-<id>/`: an item that could not be read, kept, never deleted.

Ingest (src/shareIngest.ts) runs at launch, on resume and on the native
`inboxChanged` event, one item at a time. Loss-free comes first, then no
duplicates (losing a shared item is worse than one rare duplicate). The
states, each durable in `prepared.json` before the next step (review round 2):
1. bound: record `{graph, day}` for the bound graph and today;
2. shaped: import the item's files into that graph (once; the names are
   recorded) and record the shaped block (OG transcription, src/shareShape.ts);
3. armed: inside `writeOwned`, `appendToJournalDay(day, …)` admits the day's
   journal and records `armed` (if the revision moved while that record was
   written, it is recorded again), then inserts the block and flushes it
   through the audited save path, with no further await;
4. written: recorded right after the flush returned ok;
5. commit: tombstone, then remove the item.

Recovery never acknowledges an item on content equality:
- `written`: flush the day (a no-op unless edits are pending) in its own
  graph, then commit. A later edit or removal of the block is the user's;
- recorded graph is not the bound one: keep the item silently until that
  graph is bound (no toast, no import, no write into another graph);
- anything else (bound, shaped or armed, whatever the revision is now):
  append. Within one process, the block ids the item's own append put in
  the store are remembered (identity, not content): a failed flush is
  retried on those blocks rather than appending a copy, and a flush that
  succeeded is marked `written` even if writing that marker failed.
A failed write keeps the item and shows an error toast.

Producers save a share whole or refuse it with a message: at most 32 files,
none over 64 MiB, enforced while collecting (iOS checks a file's size before
reading it). Android accepts only `content:` images from another app's
provider and decodes and validates the whole intent before any work. Each
Android share intent is one occurrence: its first receipt stamps a random id
on the intent (`page.tine.app.SHARE_OCCURRENCE`, carried across process
death in the Activity's saved state), and the item is named by it. Every
delivery of that intent (fresh, restored, from Recents) checks for `<id>` or
`.committed-<id>`: if either exists, the inbox is synced and the arrival
announced; otherwise the item is (re)published with the full barrier order.
One delivery per occurrence runs at a time; equal content shared twice is
two occurrences and two items. A redelivery whose files can no longer be
read (the sender's grant ended) is refused with a toast naming the file.

Routes stay open-only (ADR 0071): `tine://today`, `tine://search[?q=]`,
`tine://capture` and graph-less `tine://page/<name>` (current graph) open or
focus; `capture` starts editing an empty bottom block of today's journal and
writes nothing until the user types.

## Refusal table
The threat model is `specs/notes/2026-08-07-trust-model-and-threat-model-decision.md`.
Every refusal keeps the item; none refuses to open the graph.

| Refusal | In-scope threat | Outcome |
|---|---|---|
| `item.json` undecodable, over 1 MiB, unknown version, a resource name that is not a plain file name, a missing resource, more than 32 resources, or no text, link or file | producer interrupted by crash/power loss on a file system without atomic directory rename; disk error | moved to `.rejected-<id>/`, user told |
| `prepared.json` undecodable or over 4 MiB | disk error (the write is atomic) | moved aside likewise; re-preparing could duplicate a landed append |
| `.tmp-<id>/` older than 24 h | producer crashed mid-write; the user saw no confirmation | swept |
| item bound to another graph (`prepared.graph`) | honest graph switch between a write and its commit | kept silently; ingested when that graph is bound again |
| Android share whose stream is not a `content:` URI, or one Tine's own package provides | another app (malformed or hostile) asks the exported share target to copy a file only Tine can read | whole share refused, user told |
| Android share with a malformed `EXTRA_STREAM` (another Parcelable, null entry, unparcel failure) | malformed input from another app | whole share refused, user told; no crash |
| share with more than 32 files, a file over 64 MiB, an unreadable file or a non-image file (both platforms); iOS: a second different web link or an unsupported attachment | provider error; content Tine does not store | whole share refused, user told; never saved in part |
| producer write, file or staging-directory sync, or rename fails | disk error, full disk | nothing published, user told |
| producer's inbox sync fails after the publishing rename | disk error | Android: retried once; if it fails again the user is told the share may not be saved and to share it again if it does not appear, and the `.receiving-<id>` marker is kept. The visible item is still ingested normally, but ingestion is not guaranteed if another crash or power loss discards the unsynced rename first; a lost item is then reported at the next start from its marker. iOS: the item is visible and is ingested; not reported as unsaved |
| item id not `[A-Za-z0-9_-]{1,64}` | malformed producer output | ignored by prepare/commit; listing never yields it |

## Unit cost
Per share: one item directory with `item.json` (71 bytes of envelope plus
the text/title/URL; a 5-character text share is 86 bytes), the resource files
at their original size, and one `prepared.json` rewritten up to four times
(bound, shaped, armed, written; 78 bytes of envelope plus the graph root,
the day title, the shaped Markdown, asset names and the recorded revision;
a written record for a 36-character block with a 35-character root and day
`Oct 10th, 2026` is 171 bytes), i.e. 2 + resources files, all removed after
ingest, plus one empty `.committed-<id>` tombstone (0 bytes, one file) kept
30 days, and on Android one `.receiving-<id>` marker (the summary, at most
about 64 bytes) that exists only while a share is copied. The graph
receives what OG writes:
one appended block on the item's journal day (the existing whole-page save
cost on 1- and 60-block journal pages) and one asset file per image.
Ordinary edits on 1- and 60-block pages add zero inbox bytes, files or
transport. The inbox is device-local and never synced. Measured 2026-10-10
by serializing those example records exactly as the producers and
`share_inbox::prepare` write them (compact JSON); remeasured 2026-10-10 for
the round-2 record.

## Consequences
Nothing can lose an item. What can duplicate one:
- the one ingest window: the app is killed (or `prepared.json` cannot be
  written) after the journal flush reached disk and before `written` did.
  The next process appends the item again. This includes a failed flush
  whose block the editor's autosave later wrote before the process ended;
- Android: a share intent the system relaunches from Recents after its
  Activity finished, or after a reboot, comes back without its stamped
  occurrence id (the saved state is gone) and is published again; one whose
  files can no longer be read is refused with a message instead;
- Android: a redelivery more than 30 days after its commit (the tombstone
  was pruned);
- an item interrupted between importing its files and recording them is
  re-imported on retry, leaving orphan asset files (no duplicate block).
Other effects:
- an item bound to a graph the user no longer opens waits in the inbox
  until that graph is opened again; it is not moved to the current graph;
- an item that waits lands on its frozen day, not on the day it is retried;
- Android: a share delivered to a running Activity (`onNewIntent`) whose
  process dies mid-copy is not redelivered by the system and is not saved;
  the next start reports it from its `.receiving-<id>` marker, naming it;
- Apple provisioning needs the App Group on the app and extension App IDs.
