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
9. Push the frozen exact candidate, manually dispatch `ci.yml` with
   `scope=full`, and require all four full jobs to succeed on that SHA. Record
   the Actions URL and confirm it with `scripts/check-ci-evidence.mjs`. PR or
   focused CI is not release evidence. Any source/rebase/version change creates
   a new SHA and requires a new full run. See `docs/CI.md`.
10. Manually dispatch `release.yml` on that same frozen ref. Its preflight must
    verify the exact-SHA CI evidence before packaging begins. Tag only after the
    exact commit's platform builds, Linux E2E, Android, real offline Flatpak job,
    and candidate assembly pass. The tag-triggered workflow enforces the same
    CI evidence before it rebuilds/publishes release artifacts.
11. After publication, inventory the real assets and prepare issue-specific
   reporter follow-ups. Comment/closure authority remains in the canonical
   agent agreement.
12. As release housekeeping, advance `previousRelease.ref` to the tag that was
    just published, run `node scripts/check-bench-policy.mjs`, and push that
    change to `master`. Tagged-candidate preflight deliberately compares with
    the release before the candidate; ordinary post-release `master` must point
    at the newly published tag so cumulative patch-cycle drift stays visible.

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
- No release packaging or tag-triggered build may begin without a completed
  manual full-CI run whose four required jobs succeeded on the exact candidate
  SHA. A green PR/focused run or a green run for a parent commit is insufficient.
- A scenario that does not reach its intended assertions fails.
- Retries may diagnose a flake but never erase the original failure.
- If documentation or website impact needs a product decision, stop with a
  concrete proposal; do not tag or publish.
- No release command may weaken these gates to make a deadline.
