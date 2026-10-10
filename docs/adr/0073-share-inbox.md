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
- `<id>/prepared.json`: `{graph, day, markdown, assets, armed}`, written by
  the app through `device_io::atomic_write` (src-tauri/src/share_inbox.rs)
  before each step that touches the graph:
  - `graph`: the canonical root of the graph the item is bound to;
  - `day`: the journal day the item lands in, frozen when it is bound
    (`{date}` is this day);
  - `markdown`, `assets`: the shaped block and the asset files imported for
    it, recorded before any append (null/empty until then);
  - `armed`: `{before, matches}`, recorded inside the admitted read the append
    uses, immediately before the insert: the day file's content revision
    (null when absent) and the number of blocks on that day equal to the
    shaped block.
- `.trash-<id>/`: a committed item between its rename and its removal.
- `.rejected-<id>/`: an item that could not be read, kept, never deleted.

Ingest (src/shareIngest.ts) runs at launch, on resume and on the native
`inboxChanged` event, one item at a time. Loss-free comes first, then no
duplicates (losing a shared item is worse than one rare duplicate):
1. bind: record `{graph, day}` for the bound graph and today;
2. shape: import the item's files into that graph (once; the names are
   recorded) and shape the block (OG transcription, src/shareShape.ts);
3. arm and append: inside `writeOwned`, `appendToJournalDay(day, …)` admits
   the day's journal, records `armed`, inserts the block and flushes it
   through the audited save path, with no await between arming and insert;
4. commit: remove the item.

Recovery of an item with a `prepared.json`:
- recorded graph is not the bound one: keep the item silently until that
  graph is bound (no toast, no import, no write into another graph);
- never armed: append (its append never started);
- armed, and the day now holds more equal blocks than `armed.matches`: the
  append landed; flush and commit;
- armed otherwise (the landed block was edited, moved into another block's
  text, or the append never ran): append again.
A failed write keeps the item and shows an error toast.

Producers save a share whole or refuse it with a message. Android accepts
only `content:` images from another app's provider, decodes and validates
the whole intent before any work, and keeps a durable per-intent
pending/published record (`filesDir/share-state/`) so a restored or Recents
redelivery resumes a pending share and skips a settled one; a share cut
short and never redelivered is reported at the next start.

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
| producer sync or rename fails | disk error, full disk | nothing published, user told |
| item id not `[A-Za-z0-9_-]{1,64}` | malformed producer output | ignored by prepare/commit; listing never yields it |

## Unit cost
Per share: one item directory with `item.json` (71 bytes of envelope plus
the text/title/URL; a 5-character text share is 86 bytes), the resource files
at their original size, and one `prepared.json` rewritten up to three times
(97 bytes of envelope plus the graph root, the day title, the shaped Markdown
and asset names; that share's final record is 182 bytes with a 35-character
root), i.e. 2 + resources files, all removed after ingest. Android also keeps
one publication record per share (`share-state/<fingerprint>`, about 60
bytes), pruned 30 days after it settles. The graph receives what OG writes:
one appended block on the item's journal day (the existing whole-page save
cost on 1- and 60-block journal pages) and one asset file per image.
Ordinary edits on 1- and 60-block pages add zero inbox bytes, files or
transport. The inbox is device-local and never synced. Measured 2026-10-10
by serializing those example records exactly as the producers and
`share_inbox::prepare` write them (compact JSON).

## Consequences
What can still duplicate (never lose) an item, all needing a crash or kill
at a precise point:
- the landed block was edited, deleted or merged before the item was
  committed: it is appended again;
- another writer added a block exactly equal to the shaped one (same
  minute, same text) after arming and the item's own insert then never
  reached disk: the item counts as landed. This is the one narrow window
  where an item is committed without its own block; the equal block holds
  the same content.
- an item interrupted between importing its files and recording them is
  re-imported on retry, leaving orphan asset files (no duplicate block).
Other effects:
- an item bound to a graph the user no longer opens waits in the inbox
  until that graph is opened again; it is not moved to the current graph;
- an item that waits lands on its frozen day, not on the day it is retried;
- Android: a share whose process died mid-copy and whose intent never comes
  back cannot be recovered (the source is gone); the user is told at the
  next start to share it again;
- Apple provisioning needs the App Group on the app and extension App IDs.
