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
- `<id>/item.json`: `{version: 1, created, text?, title?, url?, resources: [{file, name, type}]}`
  plus the resource files. Producers write `.tmp-<id>/` and publish it with
  one directory rename, so a partial item is never visible.
- `<id>/prepared.json`: `{markdown, day, baseline}`, written by the app through
  `device_io::atomic_write` before the journal write (src-tauri/src/share_inbox.rs).
- `.trash-<id>/`: a committed item between its rename and its removal.
- `.rejected-<id>/`: an item that could not be read, kept, never deleted.

Ingest (src/shareIngest.ts) runs at launch, on resume and on the native
`inboxChanged` event, one item at a time: import resources as graph assets
through the existing asset import, shape the content (OG transcription,
src/shareShape.ts), record `prepared.json` with the count of identical root
blocks already on that day's journal, append through `appendToTodayJournal`
inside `writeOwned`, then commit (remove) the item. A crash after the journal
write and before the commit is detected on the next run by the block count
exceeding the recorded baseline, so the item is committed without a second
append. A failed write keeps the item and shows an error toast.

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
| item id not `[A-Za-z0-9_-]{1,64}` | malformed producer output | ignored by prepare/commit; listing never yields it |

## Unit cost
Per share: one item directory with `item.json` (62 bytes of envelope plus
the text/title/URL; a 5-character text share is 67 bytes), the resource files
at their original size, and one `prepared.json` (about 50 bytes plus the
shaped Markdown and day title; that share's is 87 bytes), i.e. 2 + resources
files, all removed after ingest. The
graph receives what OG writes: one appended block on today's journal (the
existing whole-page save cost on 1- and 60-block journal pages) and one asset
file per image. Ordinary edits on 1- and 60-block pages add zero inbox bytes,
files or transport. The inbox is device-local and never synced. Measured
2026-10-10 by serializing those two example records exactly as the producers
and `share_inbox::prepare` write them (compact JSON).

## Consequences
- An accepted gap: a crash between importing a resource as an asset and
  recording `prepared.json` leaves an orphan asset file (no duplicate block).
- An accepted gap: if the day rolls over between the baseline count and the
  append, the item lands on the new day, as OG's quick capture would.
- Apple provisioning needs the App Group on the app and extension App IDs.
