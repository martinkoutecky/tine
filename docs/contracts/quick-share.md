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
  `https://gist.githubusercontent.com`. At most three requests, one at a time.
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
- `snapshot` is the published snapshot, schema 1, that the published reader
  already renders (`PublishedSnapshot` in `publishedBackend.ts`; written by
  `publish_query::snapshot_at`). Its `exported_at` equals `created_at`.
- **One page shared:** `home` and `title` are that page's name. **Several:**
  `home` and `title` are the share's name, and the home page is a synthetic
  page (`path` "") whose blocks are one `[[page]]` link per shared page. The
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
any string 1 MiB.

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
