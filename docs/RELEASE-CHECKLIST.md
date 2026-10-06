# Release checklist

This is Tine's durable ship contract. `scripts/check-release-readiness.mjs`
enforces the machine-checkable parts; the canonical agent agreement defines who
may tag, publish, comment, and close issues.

## Step 0 — before the freeze

Answer two questions before spending a minute on candidate work. Both were
skipped for v0.6.984 and both cost real time: `master` was already red in hosted
CI when the release began, so the first candidate's gates were spent
rediscovering a failure that had nothing to do with the release, and every gate
had to be rerun after the fix.

0a. **Is `master` green in CI right now?**

```bash
gh run list --workflow=ci.yml --branch=master --limit=5 \
  --json conclusion,event,headSha,displayTitle,url
```

A push-triggered run is NOT full CI — it runs "Landed-code validation" only and
skips the Windows, Android, nextest-shard and performance jobs. Treat a red or
absent result as work to finish *before* the freeze, not during it.

0b. **Does the Rust job CI will run pass locally?**

```bash
source scripts/env.sh
node scripts/tine-core-nextest-contract.mjs --mode linux --run-selection
```

This is the exact selection hosted CI runs, and it is not what `cargo test -p
tine-core` runs. Running it here turns a ~20-minute hosted round trip into a
local one.

## Every release

1. Freeze the candidate and finish the version/changelog update.
   Use the dedicated coordinator `release-freeze` on the current full SHA and
   `release-heartbeat` whenever the candidate SHA or hosted Actions run changes;
   never use an ordinary batch lease. Other agents may keep working in topic
   branches, but no branch may integrate into `master` while the freeze is
   active.
   `node scripts/check-storage-pin.mjs` must bind Cargo's exact
   `tine-storage` tag/commit to the checked-in upstream certification receipt
   and persistent-format manifest, with no path, patch, branch, or rev override.
2. Generate `docs/releases/vX.Y.Z-impact.json` from every Added, Changed, and
   Fixed changelog bullet. For each item record regression coverage and four
   separate dispositions: user-facing **Guide**, engineering docs/contracts,
   website, and blog (`update`, `current`, `not-applicable`, or `consult`). A
   contract or changelog is not Guide coverage: Guide `update`/`current` must
   cite canonical files under `crates/tine-core/src/templates/`. Every
   user-visible Added item needs Guide coverage; a user-visible Changed item
   may opt out only as explicitly performance-only, packaging-only, or
   internal-only. `consult` blocks the release.
3. For every accepted bug, require an entry in the indexed regression catalogs
   before the production fix begins (UI/native in the UI inventory; other bugs
   in the non-UI inventory). Public Fixed entries reference their GitHub issue;
   internal reports use a stable catalog ID. An exemption needs substitute
   evidence and a reason.
4. Run `node scripts/check-issue-receipts.mjs` and post any receipt it names.
   Every issue the Fixed section claims must already carry a maintainer
   `fixed-on-master` receipt: root cause/invariant, the literal end-to-end path
   exercised, the sibling matrix reviewed, the platform and evidence layer, and
   anything still unverified. v0.6.984 claimed fifteen issue fixes and eleven
   had no receipt at all, so the reporters learned nothing until publication and
   the close-out had to reconstruct the evidence from commits.
5. Regenerate the canonical Guide site and prove the checked-in
   `website/guide/` output, bundled Guide pages, links, block references, and
   assets are current.
6. Run the complete Linux release E2E catalog (`npm run e2e:linux:release`)
   against the production-protocol candidate binary. Retain screenshots, DOM,
   console/backend logs, graph diff, JUnit, and JSON on failure. See
   `docs/UI-REGRESSION-TESTING.md` for the exact binary and evidence contract.
7. As soon as that frozen candidate passes its local exact-commit gates, deploy
   that exact tested artifact to `~/research/tine` without waiting to be asked.
   Record and compare the staged/deployed SHA-256 so Martin can test the actual
   release candidate while the slower platform workflows run. If using
   coordinator `stage-deploy`, pass `--release-build`; its ordinary-batch
   default is intentionally a faster nondeterministic local profile and does
   not count as release evidence. When an exact signed Android APK is available,
   also copy it to `~/research/tine.apk` and verify its SHA-256 against the
   versioned source artifact before asking Martin to test it.
8. The Windows x64 real-app smoke suite is advisory when available. Separately,
   step 9's Windows CI evidence is blocking: it compiles all `tine-core` test
   targets against the exact certified `tine-storage` pin, then runs the
   contract-selected cross-layer parity/durability/lifecycle smokes. The full
   physical storage suite belongs to a new `tine-storage` version's independent
   certification, not to ordinary Tine releases. See `docs/CI.md`.
9. Set `scripts/bench-policy.json`'s `previousRelease.ref` to the most recently
   published release (never the unshipped candidate). Do not advance the
   immutable baseline. Push the exact candidate and require the same-machine A/B
   performance job to pass; an expected budget breach is a stop/consult decision,
   not permission to weaken the budget. Also run `npm run bench:startup` against
   the immutable v0.4.7 native binary, retain its timing JSON and early-frame
   sequence, and inspect those frames for new blank, intermediate, or corrupt
   paints before shipping.
10. Push the frozen exact candidate, manually dispatch `ci.yml` with
   `scope=full`, and require all nine full jobs to succeed on that SHA,
   including the Linux nextest inventory contract and all four hash shards plus
   the blocking Windows core compile + integration-smoke job. Record the Actions URL
   and confirm it with `scripts/check-ci-evidence.mjs`.
   PR or focused CI is not release evidence. Any source/rebase/version change
   creates a new SHA and requires a new full run. See `docs/CI.md`.
11. Manually dispatch `release.yml` on that same frozen ref. Its preflight must
    use `mode=build`, `publish=false` and verify the exact-SHA CI evidence before
    packaging begins. Record the successful source run ID. Tag only after the
    exact commit's platform builds, Linux E2E, Android, real offline Flatpak job,
    and candidate assembly pass. Candidate and exact Linux/Windows proof inputs
    expire after three days.
    A tag push does not publish. On the exact candidate tag, manually dispatch
    `release.yml` with `mode=promote`, the recorded `source_run_id`, and
    `publish=true`; this verifies the immutable candidate and publishes it
    without rebuilding platforms. If an intervening descendant changes only an
    exact path registered in `scripts/release-proof-only.json`, first rehearse
    `mode=promote`, `publish=false` on that target, require every blocking
    affected proof against the retained source binary to pass, then tag and run
    the publishing promotion. Any unclassified, ambiguous, added/deleted/renamed,
    non-descendant, or product-identity change requires fresh full CI and a new
    `mode=build` candidate.
12. After publication, inventory the real assets and prepare issue-specific
   reporter follow-ups. Comment/closure authority remains in the canonical
   agent agreement.
13. As release housekeeping, advance `previousRelease.ref` to the tag that was
    just published, run `node scripts/check-bench-policy.mjs`, and push that
    change to `master`. Tagged-candidate preflight deliberately compares with
    the release before the candidate; ordinary post-release `master` must point
    at the newly published tag so cumulative patch-cycle drift stays visible.
14. On every successful publication **and every aborted release**, run
    `tine-coordination release-unfreeze --owner <release-owner>`, followed by
    `tine-coordination status`. The release cycle is not operationally complete
    until status prints `IDLE master integration is available`. The bounded
    freeze expiry is crash recovery, not a substitute for this checklist step.

## Additional `0.x.0` minor-release gates

1. Do the Reddit/blog pass locally; Reddit must never be fetched, validated, or
   artifacted by GitHub Actions or release packaging. Run
   `npm run blog:sync -- --version=X.Y.0`, review every `r/TineOutline` post by
   Martin plus every comment/reply in threads already cited by an existing blog
   entry, update `website/blog/reddit-sources.json` for new source posts, and edit
   `website/blog/` with the substantive new material. The sync script writes an
   ignored working snapshot under `test-results/reddit/`; it does not write the
   editorial prose and its output is not committed. Prefer Reddit's public
   REST/JSON feeds, never RSS/Atom. If Reddit rejects the local REST request,
   inspect the live post and comment pages directly rather than moving the work
   to a hosted runner. Run `npm run blog:check` locally when the editorial pass
   is complete.
2. Audits are not a checklist gate. The maintainer requests audits by kind
   before a major release; record any that ran, and their findings, in the
   release receipt.

## Fail-closed rules

- Missing or stale evidence is a failure, never a successful skip.
- No release build may begin without a completed manual full-CI run whose nine
  required jobs succeeded on the exact candidate SHA. A green PR/focused run or
  a green run for a parent commit is insufficient. Promotion may reuse only a
  successful no-publication candidate run through the machine-checked
  same-commit or registered proof-only path; it cannot substitute arbitrary
  parent evidence.
- Tag pushes never publish by themselves. Publication requires an explicit
  manual `mode=promote`, `publish=true` dispatch on the tag and a verified
  promotion receipt naming the actor, source/target commits, retained artifacts,
  product digest, and proof results.
- A scenario that does not reach its intended assertions fails.
- Retries may diagnose a flake but never erase the original failure.
- If documentation or website impact needs a product decision, stop with a
  concrete proposal; do not tag or publish.
- No release command may weaken these gates to make a deadline.
- A release manager must never leave `master` FROZEN after success, failure, or
  abort. Conversely, do not force-unfreeze a live release merely to land another
  branch; continue in that branch and integrate after the release disposition.
