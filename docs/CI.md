# CI and release evidence

Tine concentrates broad CI at the release boundary. Ordinary coding should use
the causal test for the changed behavior plus directly affected neighbors; it
should not repeatedly start every platform build and performance comparison.
The frozen release candidate receives the exhaustive pass.

## What runs when

| Event | Automatic work | Purpose |
| --- | --- | --- |
| Non-doc pull request | `ci` → `PR validation / Linux unit and contract checks` | TypeScript, frontend, the same curated `tine-core` nextest release selection the release shards run (unsharded, `ci` profile), plugin SDK, plus cheap generated-artifact/release contract guards. No Windows, Android, performance, Flatpak build, or release packaging. |
| Docs/image-only pull request | No app CI | Avoid runner work for prose and image-only changes. A Flatpak/website metadata PR still gets its path-specific lightweight validator. |
| Push to `master` | No app test/build workflow | Merging does not repeat CI after the reviewed commit. Website pushes may still deploy Pages; issue automation is separate from app CI. |
| Manual `ci`, scope `full` | Linux contracts/tests plus four deterministic process-isolated `tine-core` nextest shards, Windows compile-all `tine-core` targets + contract-selected cross-layer integration smoke, Android core compile, same-runner performance A/B | Required exact-SHA release-candidate evidence against the certified `tine-storage` pin. |
| Manual `ci`, focused scope | Only `windows`, `android`, `android-runtime`, `android-ui-runtime`, or `performance` | Platform/performance proof while developing relevant changes. A focused run never satisfies the release gate. |
| Manual `ui-e2e` | Complete or scenario-focused Linux/Windows real-app proof | UI/harness debugging between releases without starting ordinary full CI. |
| Manual `Flatpak build test` | Real offline Flatpak build | Focused packaging proof. The release workflow calls the same workflow as a hard gate. |
| Manual `release`, `mode=build` | Exact-SHA CI evidence check, release preflight, real Flatpak, desktop/Android packages, release E2E, candidate assembly | Expensive release proof. It fails before packaging if the exact candidate lacks successful full CI evidence. With `publish=false` it creates an immutable private candidate and receipt. |
| Manual `release`, `mode=promote` | Verify a successful no-publication source run, classify source-to-target, rerun every registered affected proof against the retained exact binary, verify candidate and promotion receipts, optionally publish | The normal same-commit publication path and the narrowly allowlisted proof-only reuse path. It never rebuilds platform packages. |

The lightweight pull-request path is a useful early signal, not release
evidence. Platform-native or observation-boundary proof remains necessary when
the changed behavior requires it.

## Frozen-candidate sequence

1. Finish release metadata and all source changes, freeze one commit, and push
   its branch.
2. Dispatch `ci.yml` on that branch with `scope=full` and wait for all nine full
   jobs, including the Linux inventory contract and each of its four hash shards.
   Record the exact commit and Actions run URL.
3. Optionally verify the same evidence from the checkout:

   ```bash
   GH_TOKEN="$(gh auth token)" node scripts/check-ci-evidence.mjs \
     --repo martinkoutecky/tine --sha "$(git rev-parse HEAD)"
   ```

4. Dispatch `release.yml` on the same frozen branch with `mode=build` and
   `publish=false`. Its preflight performs the evidence check independently
   before toolchain/dependency setup or packaging. Record the successful source
   run ID. Candidate and exact Linux/Windows proof inputs are retained for three
   days; do not postpone promotion beyond that window.
5. After the manual release matrix and candidate assembly succeed, tag that
   exact commit only with explicit release authority. A tag push alone does not
   start publication. Manually dispatch `release.yml` on the tag with
   `mode=promote`, the recorded `source_run_id`, and `publish=true`. The exact
   same-commit promotion has an empty proof delta, verifies every candidate
   byte and receipt, and publishes without rebuilding platforms.
6. If the candidate instead needs a correction confined to an exact path in
   `scripts/release-proof-only.json`, commit it as a descendant of the source
   candidate. First dispatch `mode=promote`, `publish=false` on that branch and
   require the promotion receipt and every blocking affected proof to pass
   against the retained source binary. Then tag the corrected target and repeat
   promotion on the tag with `publish=true`. The publication run deliberately
   reruns the affected proofs before publishing.

Exact-SHA remains the default rule. Any source, dependency/lockfile, generated
runtime asset, build flag, packaging input, workflow/build recipe, manifest,
add/delete/rename, unrelated-history, or unclassified change invalidates reuse
and requires fresh full CI plus `mode=build`. The sole exception is a descendant
delta accepted by the narrow registry and product-identity classifier above;
ambiguity fails closed. After a full-CI failure, rerun all jobs (not only the
failed job) so the latest workflow attempt contains fresh successful evidence
for all nine full lanes while Actions retains the failed attempt.

`scripts/check-ci-evidence.mjs` requires a completed manual `ci.yml` run whose
`head_sha` is exact and whose nine stable full-job names all concluded
`success`. That list includes all four Linux nextest shards, so a missing or
skipped shard cannot satisfy the gate. PR runs, focused runs, skipped jobs,
failed jobs, and merely green release runs cannot satisfy it.
`scripts/test-release-pipeline.mjs` keeps this fail-closed contract under
deterministic fixtures.

Proof reuse does not rewrite provenance or turn a test-only change into a new
product build. `release-candidate-receipt.json` binds
the built source commit, normalized product-input digest, and every asset's
name, size, and SHA-256. `release-promotion-receipt.json` additionally records
the exact target commit, source workflow/artifact IDs, proof-only changes,
required proof results, and authorizing GitHub actor. A blocking proof failure,
missing/expired artifact, dirty product input, candidate-byte mismatch, wrong
source pair, changed product digest, or non-canonical inventory rejects
promotion. Broadening `scripts/release-proof-only.json` is itself a product
change and requires explicit negative contract fixtures plus a fresh build.

## Windows release scope

Linux is Tine's complete behavior matrix: its nextest inventory contract proves
every selected non-ignored `tine-core` test runs exactly once across four
isolated shards. Selection is allow-by-default: every current and newly added
test enters the release gate automatically. The only subtraction is the exact
known-red legacy-oracle corpus, proven BY NAME and classified by behavior family
rather than hidden behind a module prefix:
`KNOWN_RED_SYNC_RUNTIME_FAILURE_FAMILIES` in
`scripts/tine-core-nextest-contract.mjs` lists every excluded test, and the
contract fails both when another test is omitted and when a listed name no
longer exists. The 2026-08-25 honest unfiltered run completed 2,116 tests with
2,071 passing, 45 normally failing, 41 ignored, and no hangs or timeouts; it
removed 47 stale or passing exclusions. A residual legacy-oracle failure is not
itself a current production fail-before. The same selection and profile run on
every pull request, so the PR gate and the release shards cannot drift apart.
Those tests
exercise Tine's semantic and lifecycle integration with the
exact certified `tine-storage` pin. The package's own complete Linux, Windows,
Android, format, crash-cut, and API matrix runs when a storage version is cut;
ordinary Tine releases do not pay for it again.

Windows is a deliberately narrower, blocking compatibility gate: it compiles
every `tine-core` test target against the pin and runs a declared cross-layer
smoke selection under nextest isolation. The selection contains every explicitly Windows-named core test plus the
bootstrap-capture, bootstrap-preparation, durability, and lifecycle witnesses
that caught the v0.6.90 Windows failures.

`scripts/tine-core-nextest-contract.mjs --mode windows --run-smoke` lists the
actual Windows core inventory before executing the smoke. It fails if an
explicitly Windows-named core test or declared witness is added, renamed,
removed, or omitted; it then runs that verified core/storage integration set
with nextest's zero-retry, fail-on-timeout profile. This is neither an advisory
subset nor a retry mask.

Full runtime parity for all `tine-core` tests on Windows is explicitly deferred.
Some platform-neutral core fixtures encode Unix-like file/identity assumptions;
they remain fully blocking on Linux. The complete cross-platform physical suite
is required by `tine-storage` certification before Tine can advance its pin.
Promoting a broader Windows core matrix is a separate compatibility project,
not a quiet gate expansion during a release.

## Between releases

- Run focused local tests while editing and the affected behavior family's
  real-app proof before integration when relevant.
- Dispatch `ui-e2e` for Linux/Windows harness or native UI changes.
- Dispatch manual `ci` with `scope=windows`, `scope=android`,
  `scope=android-runtime`, `scope=android-ui-runtime`, or `scope=performance`
  when that platform boundary is the thing being changed.
- `scope=android-ui-runtime` is the manual API-35 x86_64 Android WebView lane
  for the #205 responsive-chrome, #207 page-reference long-press, and #375
  initial-native-selection journeys. It uses instrumentation-injected
  MotionEvents and runs each method in a fresh app/WebView lifetime. #375 uses
  two independently reset methods: a first-line caret followed by a second-line
  hold in a wrapped editor, and a single-line hold. The
  artifact retains exact app/device/WebView identity, JUnit accounting,
  screenshot, DOM/native JSON receipt, and targeted logcat even when a method
  is red. It asserts semantic fit/menu/selection outcomes rather than fixed
  screenshot pixels; it is hardware-equivalent evidence only, not OEM WebView
  or IME coverage. A red or unrun method remains unverified and cannot support
  a release or public fixed claim.
- The frozen `scope=full` release gate includes the Android app-UID managed
  activation, crash-recovery, sharing, clean-shutdown, and reopen journey.
- Dispatch the Flatpak workflow for offline packaging changes.
- Do not dispatch `scope=full` as a routine completion ritual. It is the frozen
  release gate.
