# 0076 — Quick share keeps its account and share keys in two private app-data files

- **Status:** Proposed (Rule 8 approval: Martin 2026-10-18, D-20261018-Q1 S1; the manager finalises this record)
- **Date:** 2026-10-18

## Context

Quick share (campaign DESIGN `2026-10-quick-share`, §1.5; contract `docs/contracts/quick-share.md`) uploads an
encrypted copy of some pages to a secret gist in the user's GitHub account. The link is
`https://tine.page/s/1/#<login>/<gist_id>/<key>`; whoever holds the key reads the share. Two secrets must
therefore outlive the dialog that created them:

- the GitHub OAuth token from the device flow, which every create, check, find and delete needs;
- each share's 32-byte key, which exists nowhere else: GitHub stores only ciphertext and the key is never
  sent to it.

Where they could live:

- **In the graph.** Rejected for the key by Martin 2026-10-17 (D-20261017-QS3, IO2): a graph synced through a
  public git repository would publish every share. What the graph does get is the share's location without the
  key, `tine.share:: <login>/<gist_id>` on the export definition (D-20261018-Q1 S2), so the Exports page can
  list shares on every device. The token never belongs in a graph.
- **In an existing app-data format.** The device settings JSON (`tine-settings.json`) is written by the generic
  atomic writer with default permissions and is read by the webview; neither may hold the token or a key
  (DESIGN §3.5: secrets never reach the webview). No other device-local format is keyed by GitHub account.
- **In the OS keychain.** Possible on desktop, uneven on Android and iOS, and a second store to keep consistent
  with the share list. Not needed under the trust model: Tine does not defend against an attacker with access to
  the account's files, the same level as the graph itself
  (`specs/notes/2026-08-07-trust-model-and-threat-model-decision.md`).

OG-RULES Rule 8 requires an ADR and Martin's approval for a new persisted format.

## Decision

We will keep quick share's secrets in two device-local app-data files, never in a graph, written only through a
private variant of the audited atomic writer:

- `quick-share-account.json`: `{"v":1,"login","account_id","token","scopes","connected_at"}`. Deleted on
  Disconnect (the directory entry is synced).
- `quick-shares.json`: `{"v":1,"shares":[{"share_id","state":"pending"|"live","account_id","login","gist_id"?,
  "created_at","title","pages","key"}]}`, rewritten whole under one process-wide lock (every window shares the
  process). `share_id` and `key` are canonical base64url; `pages` is a count; `account_id` is GitHub's immutable
  numeric id, `login` a display and URL hint.

The writer is `tine_store::atomic_file::atomic_write_private`, reached only through
`src-tauri/src/device_io.rs::atomic_write_private`: the temporary is created with `create_new` and mode 0600
(`OpenOptionsExt::mode` at creation, never a later chmod) on Linux, Android, macOS and iOS; on Windows it
inherits the per-user app-data ACL; then fsync, rename over the target, directory sync. A target outside the
five refuses to write a secret rather than use default permissions. The platform list is pinned by the I-16
guard tests.

Refusals, each with its in-scope threat (contract "Device records" table):

| Refusal | In-scope threat | Recovery |
| --- | --- | --- |
| `quick-shares.json` that does not parse, or has a newer `v`, is never overwritten; share changes refuse and the list reports it | disk error or torn write by another tool, an external editor, a newer Tine on the same app data | the file is left for repair or a newer Tine (it holds keys that exist nowhere else) |
| An unreadable `quick-share-account.json` reads as not connected | disk error, external editor | Connect writes a fresh file; the token is re-obtainable |

`prepare`'s private build graph (a copy of the selected pages plus `config.edn` in a temporary directory under
app cache, removed before `prepare` returns; DESIGN §3.1) is not a persisted format. Its writer sites are listed
in `APPROVED_WRITER_SITES` as such.

**Unit cost (I-25).** Per ordinary 1- or 60-block edit: nothing (no record, file or transport). Per share, on
this device: about 250 B per row in `quick-shares.json` (269 B for a one-row file, measured by
`records_are_written_owner_only_through_an_atomic_rename` with `--nocapture`), the whole file rewritten twice
(pending, then live), so a create writes about 2 × the file size (a 60-share file is about 15 KB); 1 file. The
same record size for a 1-block and a 60-block share: a row holds ids, a title, a timestamp and a page count,
never content. Connect: one 149 B file. Transport: none (device-local, never synced). In the graph, per share:
one `tine.share:: <login>/<gist_id>` line, 57 B for an 8-character login and a 32-hex gist id, on the query
block it started from, or a 101 B definition block on the Exports page (for a page named "Project"), plus a
93 B Exports page on first use; 1 page file rewritten through the normal save (the whole file: the size of the
page that holds the definition). Sizes in the graph computed from the exact strings `src/quickShare/definitions.ts`
and `src/exportsPage.ts` write.

## Consequences

- A share's link can be copied again only on the device that made it. Another device sees the share on the
  Exports page (through `tine.share::`) and can delete it through Find shares on GitHub, but cannot open it. The
  planned opt-in "Keep share links in this graph" (DESIGN §8) would be a separate decision.
- Losing app data loses the keys: the shares stay on GitHub, findable and deletable, but no longer openable from
  Tine. Backups (ADR 0074) cover the graph, not app data.
- Two more entries in the persisted-format census (33) and five approved writer sites (two in
  `atomic_file.rs`, three scratch/in-memory sites in `tine-graph-features/src/quick_share.rs`).
- Changing either layout means a new `v`; an older Tine then refuses to overwrite the newer file instead of
  losing keys.
