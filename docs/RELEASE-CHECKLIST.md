# Release checklist

This is Tine's durable ship contract. `scripts/check-release-readiness.mjs`
enforces the machine-checkable parts; the canonical agent agreement defines who
may tag, publish, comment, and close issues.

## Every release

1. Freeze the candidate and finish the version/changelog update.
2. Generate `docs/releases/vX.Y.Z-impact.json` from every Added, Changed, and
   Fixed changelog bullet. For each item record regression coverage and its
   Guide/docs, website, and blog disposition (`update`, `current`,
   `not-applicable`, or `consult`). `consult` blocks the release.
3. For every accepted bug, require an entry in the indexed regression catalogs
   before the production fix begins (UI/native in the UI inventory; other bugs
   in the non-UI inventory). Public Fixed entries reference their GitHub issue;
   internal reports use a stable catalog ID. An exemption needs substitute
   evidence and a reason.
4. Regenerate the canonical Guide/demo site and prove the checked-in
   `website/demo/` output, bundled Guide pages, links, block references, and
   assets are current.
5. Run the complete Linux release E2E catalog (`npm run e2e:linux:release`)
   against the production-protocol candidate binary. Retain screenshots, DOM,
   console/backend logs, graph diff, JUnit, and JSON on failure. See
   `docs/UI-REGRESSION-TESTING.md` for the exact binary and evidence contract.
6. As soon as that frozen candidate passes its local exact-commit gates, deploy
   that exact tested artifact to `~/research/tine` without waiting to be asked.
   Record and compare the staged/deployed SHA-256 so Martin can test the actual
   release candidate while the slower platform workflows run.
7. Run the Windows x64 smoke suite when available. It is advisory until the
   release policy explicitly promotes it.
8. Set `scripts/bench-policy.json`'s `previousRelease.ref` to the most recently
   published release (never the unshipped candidate). Do not advance the
   immutable baseline. Push the exact candidate and require the same-machine A/B
   performance job to pass; an expected budget breach is a stop/consult decision,
   not permission to weaken the budget. Also run `npm run bench:startup` against
   the immutable v0.4.7 native binary, retain its timing JSON and early-frame
   sequence, and inspect those frames for new blank, intermediate, or corrupt
   paints before shipping.
9. Run ordinary CI plus the manual release
   workflow. Tag only after the exact commit's platform builds, Linux E2E,
   Android, and real offline Flatpak job pass.
10. After publication, inventory the real assets and prepare issue-specific
   reporter follow-ups. Comment/closure authority remains in the canonical
   agent agreement.
11. As release housekeeping, advance `previousRelease.ref` to the tag that was
    just published, run `node scripts/check-bench-policy.mjs`, and push that
    change to `master`. Tagged-candidate preflight deliberately compares with
    the release before the candidate; ordinary post-release `master` must point
    at the newly published tag so cumulative patch-cycle drift stays visible.

## Additional `0.x.0` minor-release gates

1. Run `npm run blog:sync -- --version=X.Y.0`, synchronize every reported new
   `r/TineOutline` post into the human blog, and re-check every Reddit thread
   already cited by an existing entry. Commit the clean
   `docs/releases/vX.Y.0-reddit.json` evidence.
2. Run three independent audit areas: data safety/security/privacy;
   behavioral correctness/Logseq compatibility; performance/resource
   lifecycle.
3. Add a focused change-cluster audit only when the release introduces or
   substantially rewrites a subsystem, write path, platform integration, or
   broad interaction surface. Record the decision either way.
4. Fix every verified critical/high finding. Medium/low findings may ship and
   are recorded for patch-cycle fix/defer/WONTFIX triage.
5. Freeze the tree and run the final required audits on one identical source
   fingerprint. Any source fix invalidates the sweep.

## Fail-closed rules

- Missing or stale evidence is a failure, never a successful skip.
- A scenario that does not reach its intended assertions fails.
- Retries may diagnose a flake but never erase the original failure.
- If documentation or website impact needs a product decision, stop with a
  concrete proposal; do not tag or publish.
- No release command may weaken these gates to make a deadline.
