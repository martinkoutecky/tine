# Comparison source

`rows.json` is JSON so the Node generator needs no parser dependency. Each row
has a stable `id`, section, feature label, required `tine` cell, and optional
`logseq` (file graphs), `db`, and `obsidian` cells. All prose is plain text and
is HTML-escaped. The template retains the published page's CSS and site chrome.

Tine cells contain `text` and either:

- `evidence`: a repository `path` to a Guide HTML page with `anchor` and a
  `contains` excerpt from that anchored block, or a test path (optionally with
  a `contains` excerpt). Missing files, anchors and excerpts fail generation.
- `claim: true` and a nonempty `reason`, displayed visibly as a claim.

Competitor cells contain `text`, a primary HTTPS `source`, and a calendar
`checked` date (`YYYY-MM-DD`). The generator restricts sources to the tool's
official documentation/site or GitHub organization. Omitted cells or
`{"unknown": true}` display **Unknown**. A supplied assertion without a source
or date fails generation. Unknown says nothing about feature availability.

Humans must review the meaning and scope of evidence: file existence cannot
prove a feature, and an allowed source domain cannot prove a specific claim.
Builds do not fetch the network or run the cited native tests. At stable cuts,
revisit competitor URLs and dates, and review evidence against current Guide
templates and tests. Prefer scoped positive descriptions; do not infer absence
from silence. Broad row labels are qualified by the cell's specific wording.

`npm run docs:build` finishes with `npm run compare:preview`. The standalone
preview is `artifacts/compare-page/compare.preview.html`; its only difference
from canonical output is a base URL so existing website CSS and links resolve.
`npm run test:compare` checks the committed generated fixture byte for byte.
It also checks the published page unless it is exactly the SHA-256-pinned
legacy page retained during this development lane. No other stale published
content is exempt. `npm run compare:check` is always strict and intentionally
fails until the stable cut regenerates the published page.

At the stable cut, follow `docs/RELEASE-CHECKLIST.md`: regenerate the Guide,
review evidence, run `npm run compare:build`, refresh the fixture with
`node scripts/build-compare-page.mjs --update-fixture`, and run both checks.
The package version supplies the version stamp and test-source release URLs.
The exact-legacy-page allowance can be removed after the first stable cut.
