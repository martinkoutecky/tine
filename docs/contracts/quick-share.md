# Quick share, generation 1

A quick share is an encrypted copy of some pages in a secret GitHub gist, read
in a browser at `https://tine.page/s/1/`. This contract fixes what generation 1
writes and what every generation-1 viewer reads, **forever**: the frozen
fixtures in `tests/fixtures/quick-share/g1/` must open, unchanged, in every
viewer build that serves `/s/1/`. An incompatible change to the link, the
envelope or the document starts generation 2 at `/s/2/`; `/s/1/` stays.

Code: writer `crates/tine-graph-features/src/quick_share.rs`, viewer
`src/quickShare/quickShare.ts`. Limits: `quick-share-limits.json` (one file,
read by the viewer and checked against the writer's constants by a unit test).
Design and rationale: the private design `DESIGN.md` (rev 3) of the quick-share
campaign.

## Link

```
https://tine.page/s/1/#<login>/<gist_id>/<key>[/page/<name> | /block/<uuid>]
```

- `<login>`: `[A-Za-z0-9-]{1,39}`, the GitHub login at creation. A location
  hint only: when the raw URL under it is 404, the viewer asks
  `https://api.github.com/gists/<gist_id>` once and fetches that answer's
  `files["tine-share.bin"].raw_url`, accepted only on
  `https://gist.githubusercontent.com`. At most three requests, one at a time,
  none following a redirect (`redirect: "error"`): a redirect would leave the
  trusted hosts and escape the count.
- `<gist_id>`: `[0-9a-f]{20,32}`.
- `<key>`: 32 bytes, canonical base64url (below).
- The rest is the published reader's permalink (`publishedPermalink.ts`):
  `/page/<encodeURIComponent(name)>` or `/block/<id:: uuid>`. The viewer owns
  only the rest: navigation, refresh and copied links keep
  `<login>/<gist_id>/<key>` byte for byte. A rest it cannot parse or resolve
  opens the home page.
- Everything after `#` stays in the browser. The viewer never puts it into a
  request, a log line or an error message.

## Gist

A secret gist, description exactly `Tine share (encrypted)`, holding
`README.md` (fixed plaintext, no user data), `tine-share.bin` (the envelope
text), and zero or more `a[0-9a-f]{16}.bin` extension files, which generation-1
viewers ignore (reserved for assets).

## Envelope (`tine-share.bin`)

```
K        = 32 bytes, OS CSPRNG, fresh per share
share_id = 16 bytes, OS CSPRNG (not secret)
H        = "TSH1" || 0x01 || share_id                         (21 bytes)
k_enc    = HKDF-SHA256(ikm = K, salt = share_id, info = "tine-share/1/enc", L = 32)
nonce    = 12 bytes, OS CSPRNG
C        = AES-256-GCM(k_enc, nonce, P, aad = H)                (128-bit tag appended)
file     = b64u(H || nonce || C)
P        = gzip (RFC 1952) of the UTF-8 share document
```

- `b64u` is RFC 4648 §5 base64url, unpadded. Decoders refuse padding,
  whitespace, characters outside the alphabet and non-zero trailing bits. The
  same rule decodes `<key>`, which must be exactly 32 bytes.
- The viewer accepts one trailing `\n` after the text (a text file that gained
  a final newline on the way is the same file); the writer never writes one.
- Checks, in order: text size; canonical base64url; decoded size; length ≥
  21 + 12 + 16; magic `TSH1` (else "not a Tine share"); version byte `0x01`
  (else "needs a newer viewer"); then AES-GCM authentication of `H`, nonce and
  ciphertext (else "the key does not match, or the file was changed").
- Known answer, pinned in both test suites and checked against Python
  `cryptography`: K = `tine-quick-share-fixture-key-32b`, share_id =
  `fixture-share-id`, nonce = `fixture-nonc` (ASCII), P = `the share document`
  → `VFNIMQFmaXh0dXJlLXNoYXJlLWlkZml4dHVyZS1ub25jxlR5aY52hv6DfbD-0w-Gt7EoF0a_kyGqrXYGhJDTl1KKHQ`.

## Share document

```
{"quick_share": 1, "created_at": "<RFC 3339 UTC>", "title": "<string>", "snapshot": <published snapshot>}
```

- `quick_share` greater than 1: "needs a newer viewer". Any other value, or a
  missing or mistyped field: refused as damaged. Unknown fields are ignored.
- Admission checks the type of every field the reader dereferences (pages and
  their blocks recursively, entries, backlink groups, reference counts,
  aliases, icons, and each query record's fields; the query IR inside a record
  only as an object), so a damaged document is refused before the reader
  starts, never crashes it later (`admitShape` in `quickShare.ts`).
- `snapshot` is the published snapshot, schema 1, that the published reader
  already renders (`PublishedSnapshot` in `publishedBackend.ts`; written by
  `publish_query::snapshot_at`). Its `exported_at` equals `created_at`.
- **One page shared:** `home` and `title` are that page's name. **Several:**
  `home` and `title` are the share's name, and the home page is a synthetic
  page (`path` "") whose blocks are one `[[page]]` link per shared page. Its
  name never equals a shared page's name or alias under page identity (the
  writer refuses the share name; the viewer refuses such a document). The
  selecting query is never shipped (`include_query_text` is forced off).
- Query records (`queries`) are looked up by host page, argument, dialect and
  `tine.*` block properties. Two identical macros in one page produce two
  identical records; a viewer may use either.
- Block `id` values are runtime ids, fresh on every share, except for a block
  with an `id::` property (`has_id: true`), whose id is that uuid. Links use
  only `id::` uuids.
- Known gap (generation 1, additive to fix): a `{{query}}` inside page
  properties has no baked record, so the reader shows its static-export
  notice there.

The frozen documents in the fixtures are the normative examples. The writer
golden (`writer.json`, the document with runtime ids renumbered) must match a
fresh writer run: an app-side DTO change that would alter generation 1 fails
`quick_share_generation_1_fixtures_stay_readable` instead of shipping.

## What a share holds (closure)

The writer selects the query's owner pages, removes the export rules' pages
(`public:: false`, the exports page: `export-definitions.md`), and projects the
snapshot from a **private build graph** made of only those documents and the
graph's `logseq/config.edn`. Backlinks, aliases, icons, block reference counts
and baked queries are answered by that graph, so no text of an unshared page
reaches a share. Hidden folders (`:hidden`) are never selectable. The private
graph lives in a caller-given scratch directory and is removed before
`prepare` returns. A page that reads back differently from the reviewed
selection, or that is private in the bytes actually copied, refuses the share.

## Limits (`quick-share-limits.json`)

Writer (refused at prepare, with the reason): 200 shared pages; document JSON
32 MiB; `P` 6 MiB; envelope text 8 MiB. Prepare also runs the viewer's
admission checks, so a share a viewer would refuse is never created.

Viewer (enforced while reading): envelope text 8.5 MiB; decoded envelope
6.4 MiB; gunzip aborts at 32 MiB of output; gist API answer 16 MiB; JSON depth
512; 201 snapshot pages (200 plus the home list); 256 queries; 200,000 blocks;
any string or object key anywhere in the document, the title included, 1 MiB.

Unverified: that GitHub accepts an 8 MiB gist create. The first trial measures
it, and the writer cap is lowered if it fails.

## Fixtures

`tests/fixtures/quick-share/g1/<case>/`: `link.txt`, `tine-share.bin`,
`document.json` (frozen: never rewritten) and `writer.json` (the writer
golden). Written only by `TINE_QUICK_SHARE_FIXTURES=write cargo test -p
tine-graph-features --test integration quick_share`, which adds missing cases
and refreshes goldens. Cases: `pages` (aliases, two identical macros in one
block, a query in page properties, page-property backlinks, dotted and
duplicate property keys, `id::` uuids with reference counts, an Org page, a
journal, a Unicode name, an empty page, a private page left out) and `one-page`.

## Writer side: GitHub, device records, commands (phase 2)

Code: `src-tauri/src/quick_share/` (`github.rs` transport, `records.rs`,
`mod.rs` service and commands, `quick_share_tests.rs` against a local mock GitHub). UI:
`src/components/QuickShareDialog.tsx`, `src/components/QuickShareSettings.tsx`
(Settings → Sharing), types in `src/quickShare/commands.ts`.

### Build gate

Creating shares is off until the deployed viewer passes the candidate's
generation-1 fixture corpus in a headless browser. Run
`node scripts/check-release-readiness.mjs --quick-share-only`; it fetches
`https://tine.page/s/1/viewer.json`, verifies the hashes of deployed files
used by the reader, and decrypts and renders every frozen fixture. Only a
successful check writes `artifacts/quick-share-viewer/writer-ready.env` and
the release job's `quick_share_viewer_g1_ready` output. The native build
takes `TINE_QUICK_SHARE_VIEWER_G1_READY` from that check; the value must
match a SHA-256 of this build's exact fixture files (names and bytes).
A debug build and the old launch-time `TINE_QUICK_SHARE=1` switch cannot
bypass this gate. Off: `quick_share_list` answers
`enabled: false` (the share dialog's Link stays "coming soon", Settings has no
Sharing tab) and every GitHub-touching command refuses with `disabled`.
`quick_share_forget`, `quick_share_copy_link` and `quick_share_list` stay
available, so records made while it was on remain manageable.

### Hosted reader builds and retention

`npm run build:quick-share-viewer` builds and checks the production reader,
then writes `artifacts/quick-share-viewer/quick-share-viewer-g1.zip` and its
SHA-256 sidecar. `viewer.json` carries `generation`, `link`, `envelope`,
`document`, `snapshot_schema` (all 1), `built_from`, `fixture_sha256`, and a
`files` map of relative names to SHA-256. The map covers every file except
`viewer.json` itself; the pinned zip digest authenticates that manifest too.
JavaScript, CSS and fonts have content-hashed names. The reader starts no
plugins, applies the generation-1 CSP and no-referrer policy, and turns
authored media into external links or not-included placeholders before
mounting resource elements. The baked-in `src/quickShare/blocked-gists.json`
refuses listed gist ids before any GitHub request. Abuse reports go to the
published support address, support@tine.page; the contact never includes the
share's fragment automatically.

`website/s/generations.json` lists each generation's `current` and `retained`
builds. A pin has a release `tag` or first-trial Actions `run_id`, plus the
zip `sha256`. Current must be retained. Pages verifies zip and file hashes,
then merges retained builds before current. Only `index.html` and
`viewer.json` may change bytes under an existing name; any asset-name
collision refuses. The deployed retention list may never lose a zip digest,
including on rollback. A trial workflow artifact expires after 90 days:
before then, archive its exact zip as a release asset and change that pin's
source to the release tag, retaining its original SHA-256. Never rebuild a
retained archive to replace its bytes. A null current means the first
viewer has not been pinned; it deliberately deploys no generation yet.

### GitHub transport

- `reqwest` 0.13 without its `json`/`form` features (bodies are built and read
  by hand, bounded): native-tls + `system-proxy` on Windows; rustls (ring
  provider) + `system-proxy` on Linux, macOS, Android and iOS. A target outside
  the five does not compile (`compile_error!`). Pinned by
  `quick_share_tls_is_declared_for_every_shipped_target` and
  `quick_share_platform_cfgs_name_all_five_targets`.
- Every request: `User-Agent: Tine/<version>`, `X-GitHub-Api-Version:
  2022-11-28`, connect timeout 15 s, read timeout 60 s, no redirects, HTTPS
  only. Bodies are read through a bound: 64 KiB from the OAuth endpoints, 4 MiB
  from the API, the envelope cap + 1 from raw reads.
- The token is a newtype that prints `<redacted>`; only the transport reads it.
  OAuth bodies never enter an error; GitHub's other messages are cut to one
  line of at most 300 characters with the token and keys removed.
- Errors (the `kind` a command rejects with): 401 → `reconnect`; 403/429 with
  `Retry-After`, or `x-ratelimit-remaining: 0` with `x-ratelimit-reset` →
  `rateLimited{minutes}` (a bare 429: 1 minute); other 403 → `github{message}`;
  a request that never left (DNS, connect, TLS) → `network`; 5xx, timeouts and
  malformed answers → `unavailable`.

### Device records

App data (`app_data_dir`), never in a graph (ADR 0076; Rule 8 approval Martin
2026-10-18, D-20261018-Q1 S1). The graph holds only where a share is, without
its key (below, "In the graph").

- `quick-share-account.json`: `{"v":1,"login","account_id","token","scopes","connected_at"}`.
- `quick-shares.json`: `{"v":1,"shares":[{"share_id","state","account_id","login","gist_id"?,"created_at","title","pages","key"}]}`.
  `share_id` and `key` are canonical base64url; `pages` is the number of shared
  pages; `state` is `pending` or `live`.
- Both are written by `atomic_write_private`: a temporary created owner-only
  (mode 0600 at creation on Linux, Android, macOS, iOS; on Windows the per-user
  app-data ACL), fsync, rename, directory sync. Every read-modify-write of the
  shares holds one process-wide lock (all windows share the process).
- Prepare draws the share id (the record id) and answers it as `shareId`, so the
  webview can mark the query block before the upload (below, "In the graph");
  its base64url spelling is never all lowercase hex of 20 to 32 characters (a
  gist id's shape; `fresh_share_id` draws again). Create seals under that id.
- Create: the payload is consumed first (a second create with the same
  `prepareId` is `previewExpired`); the row is written `pending` before
  `POST /gists`; 201 → `live` with the gist id. A request that never left, or a
  4xx answer (a definite non-creation), removes the row. Any other outcome keeps
  it `pending` and the command rejects with `maybeUploaded{shareId}`. Nothing is
  retried automatically.
- Check, Delete and Find run only for the connected `account_id`. A 404 on
  Delete answers `gone` (shown "No longer on GitHub"; not persisted); the key
  goes only on Forget.
- No command result, error or log line carries the token or a key: a link is
  written to the clipboard natively (`quick_share_create`,
  `quick_share_copy_link`) and never returned.

| Refusal | In-scope threat | Recovery |
| --- | --- | --- |
| `quick-shares.json` that does not parse, or has a newer `v`, is never overwritten; every share change refuses and the list reports it | disk error or torn write from another tool, an external editor, a newer Tine on the same app data | the file is left for repair or a newer Tine; it holds keys that exist nowhere else |
| An unreadable `quick-share-account.json` reads as not connected | disk error, external editor | Connect writes a fresh file; the token is re-obtainable |
| A token for a superseded, cancelled or disconnected sign-in is dropped, never stored | concurrent honest instances (two windows), a late answer from GitHub | the newest attempt connects |
| No poll after the device code's `expires_in` (elapsed time from its arrival, network waits included); a token or account lookup completing past it is dropped, never stored | a slow or stalled network, a late answer from GitHub | Connect again (a fresh code) |
| Check and Delete for another `account_id` | multi-device divergence (records made with another account) | connect that account |
| A found gist without the marker, or owned by another user, is not deleted | malformed input from the gist listing | none needed |

Unit cost (I-25), measured by `records_are_written_owner_only_through_an_atomic_rename`
(`--nocapture` prints it): per graph edit nothing. Per share (records; the graph
side is under "In the graph"): two rewrites of `quick-shares.json` (pending, then live), 250 B per row
(269 B for a one-row file; a 60-share file ≈ 15 KB, rewritten whole, so a
create costs ≈ 2 × file size), 1 file; one gist create of the envelope (≈ 1.37 ×
`P`) plus the fixed README. Connect: one 149 B file. Transport per graph edit:
none (shares are made on request).

### Re-share in place

The page marked `tine/exports:: true` offers **Update all** beside its title,
including after renaming and in other panes. It finds definitions with the
ordinary `(or (property export) (property tine.share))` query and parses their query macros with lsdoc.
Each distinct hosted share with a device-local key uses `quickSharePrepare`
with its existing record id. Pending definitions, missing keys, duplicate
destinations, ambiguous queries and preparation failures are listed as skipped
with a reason. An incomplete definitions query refuses the batch, rather than
silently presenting a partial list as complete. Folder definitions join the same
preview through the read-only query export plan, with their recorded leaf and
Replace mode. They show page counts and left-out reasons and use the existing
fingerprint-checked publisher on confirmation (see `export-definitions.md`).

All preparations finish before confirmation is available. One combined preview
shows every title, and for quick shares membership counts and collapsible changed page names. Confirm
consumes each successful preview once through `quickShareCreate`, sequentially,
with the same native locking, revision re-read, PATCH and unknown-outcome rules
as single Re-share. A failure does not stop later shares, and every outcome is
shown; there is no automatic retry. Closing or switching graphs discards unused
previews and prevents further uploads; an upload already started may finish.
No unchanged-content optimization is used: unchanged membership can still carry
edited content, so every successfully prepared share is updated on confirmation.

The shared preview pool is bounded by total payload bytes: four writer-limit
payloads (24 MiB), plus a bookkeeping cap of 64 entries; entries expire after
ten minutes. Overflow refuses the new preparation without evicting any
reviewed payload. The batch shows that refusal for affected shares; closing
discards the retained payloads, and individual Re-share can handle skipped
definitions. This changes neither `quick-shares.json` nor in-graph properties.

Share again from a query carrying a live `tine.share::` value, or choose
Re-share beside Delete in the Exports overview. The local record is matched
by gist id, regardless of login spelling. Without its device-local key the
dialog explains why and offers **Re-share as a new link**, preserving the old
definition and share. An unreadable records file refuses instead of pretending
the key is absent.

`quick_share_prepare(request, shareId?)` validates the new snapshot through
the ordinary prepare path. With `shareId`, native code reads the owned Tine
gist's current history revision and its revision-pinned raw envelope, decrypts
with the local K, checks the authenticated share id, and reads the snapshot's
real pages (nonempty path; the synthetic home is excluded). Comparing page
identity keys against the new selection gives **+N new / −M gone**, including
names in a collapsible list. The previous membership cannot come from the
record's page count. This uses no record or in-graph format change.

The reviewed revision and exact prepared bytes stay in memory under the
existing 10-minute/4-preview/discard limits. Confirmation consumes the preview
once. Re-share serializes per share id within the process (the record lock is
process-wide only; there is no cross-process lock to reuse). It seals under
the same K and share id with a fresh OS-random 96-bit nonce, then re-reads
the current gist revision immediately before `PATCH /gists/{id}`. No
undocumented conditional header is sent. Only `tine-share.bin` is replaced.
Success updates the existing row's title/page count, preserves its key, login,
creation date and gist id, and copies the unchanged link natively. No graph
write or duplicate definition is needed for in-place Re-share.

| Update result | Outcome and recovery |
| --- | --- |
| Valid 200 response for the same gist | Confirm updated; same link |
| Gist gone (404) | `notFound`; keep key and graph definition; never silently create another gist |
| 401 / other 403 / rate limit | Existing `reconnect` / sanitized `github` / `rateLimited`; keep record and graph |
| Revision or local record changed since review | `shareChanged`; upload nothing; Refresh for a fresh preview |
| Request known not sent (DNS/connect/TLS) | `network`; no automatic retry; Refresh before another attempt |
| Timeout, dropped response, 5xx, malformed or unexpected success, local record-save failure after PATCH | `maybeUpdated`; never retry automatically or claim success; Review current share or Settings Check reads and decrypts GitHub again |

**Residual race:** another writer can update between the final re-read and
PATCH. GitHub's gist update has no documented compare-and-swap. Whichever
PATCH wins leaves one complete valid snapshot; no graph data is changed or
lost. Re-share again if needed. Check on a live record now reads and decrypts
the current gist and refreshes its title/page count; the next Re-share derives
its membership from the current snapshot and shows that copy's timestamp;
Settings Check is available on live rows too and reports the current copy's
timestamp and page names. This resolves an unknown outcome
without a new record state. GitHub keeps earlier revisions, so Re-share does
not remove previously shared text; only Delete removes the hosted gist.
Readers may see raw-content cache delay (about five minutes).

Cost per Re-share: preview one authenticated gist GET plus one bounded raw
read/decrypt; confirm one gist GET and one complete-envelope PATCH, followed
by at most one private atomic record rewrite (only if metadata changed).
Zero graph saves, and no work on ordinary edits. Check uses one gist GET/raw
read and at most one record rewrite. Membership and locks are memory-only.

### In the graph (D-20261017-QS3, D-20261018-Q1, D-20261018-QS3)

A share is an export definition (`docs/contracts/export-definitions.md`): a
query block carrying `tine.share:: <login>/<gist_id>`. Never the key. While its
upload is in flight or unconfirmed, the same block carries
`tine.share:: pending/<record id>` (the row's `share_id`; not secret, it is in
the envelope header). Written by the webview through the document layer's
ordinary block edits and its guarded save (`src/quickShare/definitions.ts`; no
new write path):

- **Start** (Create pressed): the marker goes on the query block the dialog
  started from, when that block can be edited, still carries the query the share
  is made from and has no `tine.share::` yet (neither another share nor another
  pending one); otherwise a new definition (the dialog's selector or query plus
  the marker) is appended to the Exports page, created on first use. The marker
  is saved before `quick_share_create` is called. If it does not reach disk, the
  share does not start: the dialog says so, the marker is taken back out of the
  working set (the property removed, an appended definition deleted) so no later
  save writes it, and the preview stays for another try. Closing the dialog
  while the marker is saved cancels the share before its upload and removes the
  marker.
- **Finish**, found by the property (`(property tine.share)` plus the page the
  marker was saved to), never by a remembered position or text, so an edited or
  moved block keeps its share: on 201, or Check finding the gist, every block
  carrying `pending/<record id>` gets `<login>/<gist_id>` (a copied block or a
  synced duplicate is another definition of the same share). A definite failure
  (any error but `maybeUploaded`) removes the marker; `maybeUploaded` keeps it.
  No block carrying it (deleted meanwhile): the share stands and the dialog says
  nothing was written to the graph.
- **Settings → Sharing** lists the bound graph's pending markers through the
  same `(property tine.share)` query, by record id, with their pages. Check
  finishes those whose record is on this device: a `pending` row is first looked
  for on GitHub (a gist created but never acknowledged), a `live` row's gist is
  written. A row of another account is left as it is. An id with no record here
  shows "Pending on another device, or no longer recorded here" and offers to
  remove the marker, after a confirmation (Tine cannot tell another device's
  share in flight from a record lost here: the record format does not name a
  device). The row's own Check finishes its marker too.
- **Delete** (204) and **Forget** remove the property from every block whose
  value names that gist, whatever login prefix it carries (an account rename
  keeps the gist id; gist ids are unique on GitHub), and the row's own pending
  marker, loading pages that are not open. A pending marker is never read as a
  gist (a record id never has a gist id's shape). A share GitHub no longer has
  (`gone`) keeps it until Forget, like its key. A gist deleted from Find shares
  on GitHub is removed the same way.
- This bookkeeping belongs to the graph the action started on, not to the view:
  closing the dialog or Settings while GitHub answers still writes it; a graph
  switch cancels it, and the marker stays for Settings → Check in that graph.
  The graph binding is rechecked before every edit and after every await, so it
  never edits a replacement graph.
- The Exports page's overview query `(or (property export) (property
  tine.share))` lists them, pending markers included; Settings → Sharing keeps
  the account, Find shares on GitHub and this device's keys (Copy link, Check,
  Delete, Forget).
- Share definitions render **Delete** in their query action bar, including
  when listed by the Exports overview. The existing confirmation and native
  delete command apply. With a local record, the connected account must match
  its immutable account id; without a key, the native command verifies the
  gist's Tine marker and owner before deleting it. A declined confirmation
  sends nothing. A `gone` result keeps the marker and key, as in Settings.
  Confirmed deletion removes graph markers through the same guarded save as
  Settings; a cleanup failure is shown while the hosted deletion stands.
- A failure to write or remove the property is shown; the share itself (created,
  deleted or forgotten) stands.

| Refusal | In-scope threat | Recovery |
| --- | --- | --- |
| A share whose pending marker did not reach disk is not uploaded | disk error or torn write of the page file, a conflicting external editor or sync delivery of that page | the dialog names the cause; Create again once the page saves |

Unit cost (I-25): per ordinary 1- or 60-block edit nothing. Per share: the
marker line `  tine.share:: pending/<22-character id>`, 46 B in a Markdown page,
written at the start and then replaced by the share's line, 57 B for an
8-character login and a 32-hex gist id (net 57 B, as before); or a 90 B
definition block on the Exports page (a page named "Project") becoming 101 B,
plus a 93 B Exports page on first use. Computed from the exact strings written.
One page file rewritten twice through the normal save (the whole file each
time: at the start, and at the finish), where a share without the marker
rewrote it once; with a synced graph, that one file is the transport, twice. A
share that fails definitely: the same two rewrites, net 0 B. The same for a
1-block and a 60-block share (a definition is one block whatever the share
holds). Delete/Forget: the same line removed from each page that carries it.

### Commands

`quick_share_prepare(request, shareId?)`, `quick_share_create(prepareId)`,
`quick_share_discard(prepareId)`, `quick_share_list()`,
`quick_share_check(shareId)`, `quick_share_find()`,
`quick_share_delete(shareId | gistId)`, `quick_share_forget(shareId)`,
`quick_share_copy_link(shareId)`, `quick_share_connect_start()`,
`quick_share_connect_status(attemptId)`, `quick_share_connect_cancel(attemptId)`,
`quick_share_disconnect()`. Host commands, not plugin capabilities (ADR 0052).
A prepared payload lives 10 minutes, within a 24 MiB total (64 at most), and goes when the
dialog closes.
