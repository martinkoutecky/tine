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

Creating shares is off in shipped builds until the viewer is deployed
(DESIGN §5 phase 3). On only in a debug build, or when Tine starts with
`TINE_QUICK_SHARE=1` in its environment. Off: `quick_share_list` answers
`enabled: false` (the share dialog's Link stays "coming soon", Settings has no
Sharing tab) and every GitHub-touching command refuses with `disabled`.
`quick_share_forget`, `quick_share_copy_link` and `quick_share_list` stay
available, so records made while it was on remain manageable.

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

### In the graph (D-20261017-QS3, D-20261018-Q1)

A share is an export definition (`docs/contracts/export-definitions.md`): a
query block carrying `tine.share:: <login>/<gist_id>`. Never the key. Written by
the webview through the document layer's ordinary block edits and its guarded
save (`src/quickShare/definitions.ts`; no new write path):

- **Create** (201, or Check finding the gist): from an unchanged query block, the
  property goes on that block (beside any `export::`); otherwise, or when that
  block already names another share, a new definition (the dialog's selector or
  query plus the property) is appended to the Exports page, created on first
  use. Written only once the gist exists: a pending share has no gist id, so no
  property.
- **Delete** (204) and **Forget** remove the property from every block that
  carries that exact value, loading pages that are not open. A share GitHub no
  longer has (`gone`) keeps it until Forget, like its key. A gist deleted from
  Find shares on GitHub removes `<connected login>/<gist_id>`.
- The Exports page's overview query `(or (property export) (property
  tine.share))` lists them; Settings → Sharing keeps the account, Find shares on
  GitHub and this device's keys (Copy link, Check, Delete, Forget).
- A failure to write or remove the property is shown; the share itself (created,
  deleted or forgotten) stands.
- A share confirmed by Check in Settings after a restart (the dialog that knew its
  query is gone) is not added to the graph; its key row stays in Settings.

Unit cost (I-25): per ordinary 1- or 60-block edit nothing. Per share: one
property line, 57 B for an 8-character login and a 32-hex gist id, on the
starting block, or a 101 B definition block on the Exports page (a page named
"Project"), plus a 93 B Exports page on first use; computed from the exact
strings written. One page file rewritten through the normal save (the whole
file); with a synced graph, that one file is the transport. Delete/Forget: the
same line removed from each page that carries it.

### Commands

`quick_share_prepare(request)`, `quick_share_create(prepareId)`,
`quick_share_discard(prepareId)`, `quick_share_list()`,
`quick_share_check(shareId)`, `quick_share_find()`,
`quick_share_delete(shareId | gistId)`, `quick_share_forget(shareId)`,
`quick_share_copy_link(shareId)`, `quick_share_connect_start()`,
`quick_share_connect_status(attemptId)`, `quick_share_connect_cancel(attemptId)`,
`quick_share_disconnect()`. Host commands, not plugin capabilities (ADR 0052).
A prepared payload lives 10 minutes, at most 4 at a time, and goes when the
dialog closes.
