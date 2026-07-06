# Plan — App-store distribution (F-Droid + Google Play)

**Status:** not started (Martin promoted both to the top of Next, Jul 6 2026). Ship
order below. This is the resume-from-cold spec.

## Eligibility snapshot (verified Jul 6 2026)

Tine's Android build is **F-Droid-clean** — no blockers on the licensing/dependency
side, so the work is packaging + process, not code surgery:

- **License:** AGPL-3.0 (`LICENSE`) — FOSS, F-Droid-eligible.
- **No proprietary deps.** Android Gradle deps are all Apache-2.0: `androidx.*` and
  `com.google.android.material:material` (Material Components — FOSS despite the
  `com.google` package name, github.com/material-components/material-components-android).
  No Firebase / GMS / play-services / crashlytics. (The other `com.google.*` strings in
  `GraphFolderPickerPlugin.kt` / `WryActivity.kt` are URI-authority / webview-package
  constants, not dependencies.)
- **No runtime self-update / telemetry on Android.** `tauri-plugin-updater` is under
  `[target.'cfg(not(any(target_os="android",target_os="ios")))'.dependencies]` in
  `src-tauri/Cargo.toml` — desktop-only. So no F-Droid `UpdateCheck` AntiFeature and no
  non-free-network concern.
- **Local-first, no data collection** → the Play "Data safety" form and F-Droid
  AntiFeatures are both trivial (declare nothing collected).

Current build reality: `release.yml`'s `android` job builds a **signed APK** (arm64) on
`v*` tags (`npx tauri android build --target aarch64 --apk`), keystore from GH secrets
`ANDROID_KEYSTORE_*`. Play needs an **AAB**; F-Droid builds from source (ignores our APK)
unless we opt into reproducible-build verification.

---

## Track A — F-Droid (do first; cheapest real distribution)

Two sub-paths. **A1 (self-hosted repo)** is hours of work and fully in our control —
do it first for immediate distribution. **A2 (main f-droid.org repo)** is the real goal
(searchable in the F-Droid client) but is weeks of build-recipe iteration because Tauri
(Rust + npm) must build **offline** on F-Droid's servers.

### A1 — Self-hosted F-Droid repo (fast path, full control)

Users add a repo URL (or scan a QR) in the F-Droid client; we publish our own
**self-signed** APKs on our own cadence, no review.

1. `pipx install fdroidserver` on the build box (or a laptop). Needs `apksigner`
   (Android build-tools, already installed per `tine-android-build`) + `aapt`.
2. `mkdir tine-fdroid && cd tine-fdroid && fdroid init` — generates a repo keystore
   (BACK IT UP; it's the repo's identity, distinct from the app signing key).
3. Drop the signed release APK(s) into `repo/` (e.g. `Tine_0.4.x_android-arm64.apk`
   from the GitHub release), then `fdroid update --create-metadata` → builds `index-v2`.
4. Add richer metadata in `metadata/dev.tine.app.yml` (Name, Summary, Description,
   License AGPL-3.0-only, SourceCode, IssueTracker, Categories) + re-run `fdroid update`.
5. Host the `repo/` directory statically. Easiest: a `fdroid/` path on **tine.page**
   (GitHub Pages) or a `gh-pages` branch — it's just static files. Add the repo URL +
   QR to the website's download section and README.
6. Automate: a CI step (or a small script) that, on each `v*` tag, downloads the release
   APK, runs `fdroid update`, and commits/pushes `repo/`. Repo signing key lives in a
   secret.

**Deliverable:** "Add our repo to F-Droid" instructions on tine.page. Users get
auto-updates through the F-Droid client from our own signature.

### A2 — Main f-droid.org repository (recipe VALIDATED end-to-end Jul 6 2026)

**A full `fdroid build` succeeds locally** (fdroidserver 2.4.5, run in the dev container).
`fdroid lint` → `fdroid` scanner → the recipe (rustup wasm target → `npm ci` →
`npm run build:wasm` rebuilding the parser **from source** → `vite build` → Rust/Android →
Gradle) → **`1 build succeeded`**, output `unsigned/dev.tine.app_4000.apk` (24.8 MB), which
aapt confirms is `versionCode=4000 versionName=0.4.0` (matches the metadata, so fdroid's
version check passed). So the recipe below is proven, not theoretical. What's left is
process: point the metadata at the latest complete release tag (v0.4.2) and open the MR from
a GitLab account. Remaining gates that only F-Droid's server settles: gradle-wrapper.jar hash
verification (routine) and, if we later want it, reproducible-builds.

**Gotchas hit while validating (documented so the buildserver run is smooth):**
- **Scanner does NOT flag the vendored base64 wasm** — so `scandelete` on it errors as
  "unused" (fdroid only counts scandelete as used when it deletes a *flagged* file). Don't
  list the wasm in `scandelete`; the recipe's `npm run build:wasm` gives from-source
  provenance regardless. gradle-wrapper.jar is auto-removed by the scanner (routine).
- **Tauri needs a `gradle` on PATH.** The scanner removes `gradle-wrapper.jar`, so `./gradlew`
  can't run and Tauri falls back to system `gradle`. F-Droid's buildserver provides one; for a
  local run, install Gradle 8.14.3 (per `gradle-wrapper.properties`) on PATH.
- (dev-container only, not F-Droid) `~/.gitconfig`/`~/.git-credentials` are read-only NFS
  mounts with `credential.helper=store`, which made fdroid's clone fail
  ("unable to write credential store: Device or resource busy"). Fixed by pointing git at a
  writable temp config: `GIT_CONFIG_GLOBAL=<tmp> GIT_CONFIG_SYSTEM=/dev/null` with
  `credential.helper=` empty. Also pre-clone into `build/dev.tine.app` to satisfy
  fdroidserver's `SOURCE_DATE_EPOCH` step. Reusable harness: `/aux/koutecky/logseq/fdroid-work/`.

### A2 details

F-Droid builds **from source** on their own build server; our GitHub APK is ignored
(unless we later add reproducible-build verification). The whole difficulty is getting a
Tauri (Rust + npm) app to build cleanly there. **Exact facts for the recipe** (verified
Jul 6 2026): version `0.4.0` → **versionCode `4000`** (Tauri scheme `major·1e6 +
minor·1e3 + patch`, per `release.yml`), **NDK `26.3.11579264`**, `compileSdk 36`,
`build-tools 35.0.0`, `minSdk 24`, **Node 20**, `frontendDist: ../dist`,
`beforeBuildCommand: npm run build`, tag `v0.4.0`.

**Blockers discovered in the repo (fix in the tine repo FIRST):**

- **Tauri leaves the Android glue gitignored.** `src-tauri/gen/android/app/.gitignore`
  excludes `tauri.properties` (→ versionCode/Name), `tauri.build.gradle.kts` (defines the
  ABI product-flavors incl. `universal`), `proguard-tauri.pro`, `assets/tauri.conf.json`,
  and `src/main/**/generated/` (WryActivity etc.). So a clean checkout is NOT
  gradle-buildable as-is — the build **must run `npx tauri android build`**, which
  regenerates all of these. (This is why the recipe drives the Tauri CLI, not `gradle:`.)
- **versionCode source.** The Tauri CLI regenerates `tauri.properties` from
  `tauri.conf.json` version using the same `4000` scheme, so the built APK's versionCode
  will be 4000 — declare that in metadata. (Belt-and-suspenders: have the recipe write
  `tauri.properties` before the build, mirroring the CI step.)
- **Scanner risk (the usual Tauri/JS wall).** F-Droid's `fdroid scanner` rejects
  prebuilt/non-free binaries in the tree. A Vite/esbuild frontend pulls **native binaries**
  into `node_modules` (esbuild ships a platform binary; rollup/swc too). Expect scanner
  hits — resolve with `ScannerExclude`/`scandelete` for `node_modules` build-time-only
  binaries, and confirm nothing non-free ends up **inside the APK** (only our JS bundle +
  Rust `.so` should). This is the part that takes iteration.
- **Rust toolchain + targets.** Install the needed targets **in the recipe**
  (`rustup target add aarch64-linux-android wasm32-unknown-unknown`), NOT via a repo-wide
  `rust-toolchain.toml`. (We tried a `rust-toolchain.toml` pin and it broke the release CI —
  see the finding below — so it was removed; F-Droid uses the buildserver's stable rust.)

**Phase-1 findings (validated on the uni box, Jul 6 2026):**

- ⚠️ **A repo-wide `rust-toolchain.toml` was tried and REMOVED.** Pinning `channel = 1.96.0`
  broke v0.4.1's macOS + Windows-arm release builds: the CI installs each cross-target onto
  `@stable` (via dtolnay), but the repo pin forced a *different* toolchain that lacked them.
  Fix: no toolchain file; the F-Droid recipe installs `aarch64-linux-android` +
  `wasm32-unknown-unknown` explicitly. (Android built fine even with the pin because that
  target was in the file's list — which is exactly why the diagnosis was unambiguous.)
- ✅ **Release build is green.** Exact output path confirmed:
  `src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk`
  (24.8 MB, arm64). The `universal` flavor comes from the generated `tauri.build.gradle.kts`.
- ⚠️ **npm swallows the CLI flags.** `npx tauri android build --target aarch64 --apk` loses
  the flags because the repo has a `"tauri": "tauri"` npm script. The recipe MUST call the
  binary directly: `./node_modules/.bin/tauri android build --target aarch64 --apk`.
- ✅ **Source tree is scanner-clean** except `src-tauri/gen/android/gradle/wrapper/gradle-wrapper.jar`
  — the standard Gradle wrapper, which F-Droid verifies against known hashes (routine, not a
  blocker). No committed `.so`/`.wasm`/`.node`/`.exe`.
- 🔴 **THE Tauri-specific blocker — the wasm parser is a vendored prebuilt.**
  `src/render/wasm/lsdoc_wasm_bytes.ts` (462 KB) is the base64-inlined wasm; `npm run build`
  only *checks a pin* (`scripts/check-wasm-pin.mjs`), it never rebuilds. F-Droid forbids
  shipping prebuilt binaries, so the recipe must **rebuild it from source** with
  `npm run build:wasm` (→ `wasm-pack build crates/lsdoc-wasm`) and `scandelete` the vendored
  copy so F-Droid never ships our blob.
  - ✅ **Verified the from-source rebuild is byte-identical** to the committed bytes (same
    sha256, empty git diff) with the pinned toolchain + wasm-pack 0.15.0 — so the parser is
    genuinely reproducible-from-source (also good for later reproducible-builds).
  - `crates/lsdoc-wasm` git-deps `lsdoc` (`github.com/martinkoutecky/lsdoc`, tag `v0.4.2`,
    **public + AGPL**, Cargo.lock-pinned) — F-Droid can fetch it.
  - ⚠️ **wasm-opt is itself a prebuilt binary wasm-pack downloads** (binaryen, into
    `~/.cache/.wasm-pack/`). F-Droid's build must provide it (`sudo apt-get install binaryen`
    → system `wasm-opt`) OR disable it via `[package.metadata.wasm-pack.profile.release]`
    `wasm-opt = false` in `crates/lsdoc-wasm/Cargo.toml` (smaller-opt wasm, no download — but
    then the bytes differ from the GitHub build, which matters only for reproducible-builds).
    Resolve during the buildserver iteration.

**Remaining steps:**

1. ✅ **Repo prep — done.** No toolchain file needed (targets installed in-recipe); the Tauri
   CLI regenerates the gitignored gen glue on build (verified); wasm rebuilds byte-identical
   from source.
2. ✅ **Metadata written + recipe validated.** `fdroid lint`/scanner/`fdroid build` all pass
   locally (fdroidserver 2.4.5) → valid unsigned APK `dev.tine.app_4000.apk`
   (`versionCode=4000 versionName=0.4.0`). Harness kept at `/aux/koutecky/logseq/fdroid-work/`
   (venv + `fdroiddata/` + `run-build.sh`) to re-run after any change. **Re-validate at the
   v0.4.2 tag** (versionCode 4002, no toolchain file) before the MR.
3. **Point the metadata `commit:`/`versionCode:` at the latest complete release tag** (v0.4.2 /
   4002). No special repo content is required anymore (the toolchain pin was removed).
4. **Open the MR** to `gitlab.com/fdroid/fdroiddata` (needs a GitLab account — Martin's
   action): fork, add `metadata/dev.tine.app.yml` (real-MR version below, with `sudo:` deps),
   push, open MR, respond to the reviewer. After merge the first build can take days–weeks;
   then `UpdateCheckMode: Tags` auto-proposes each new tag.
5. **(Later, optional) Reproducible builds** — add `Binaries:` pointing at our GitHub
   release APK so F-Droid ships **our** signature (sideloaded users update in place).
   Needs a deterministic release build (fixed timestamps, sorted zip). Skip for v1.

**Starter `metadata/dev.tine.app.yml`** (verify paths/flavors against the local build):

```yaml
Categories:
  - Writing
License: AGPL-3.0-only
AuthorName: Martin Koutecký
SourceCode: https://github.com/martinkoutecky/tine
IssueTracker: https://github.com/martinkoutecky/tine/issues
Changelog: https://github.com/martinkoutecky/tine/blob/master/CHANGELOG.md

AutoName: Tine
Summary: Fast local-first Logseq-compatible outliner
Description: |-
    Tine is a fast, local-first outliner that reads and writes a real Logseq
    Markdown/Org graph on disk. (Fill from README.)

RepoType: git
Repo: https://github.com/martinkoutecky/tine.git

Builds:
  - versionName: 0.4.2            # latest complete release (0.4.1 missed macOS + win-arm)
    versionCode: 4002             # 0.4.2 → 4*1000+2  (Tauri scheme major·1e6+minor·1e3+patch)
    commit: v0.4.2
    sudo:
      - apt-get update || true
      - apt-get install -y nodejs npm binaryen   # binaryen = system wasm-opt
    ndk: 26.3.11579264
    build:
      - cargo install wasm-pack --version 0.15.0 --locked   # buildserver has no wasm-pack
      - rustup target add aarch64-linux-android wasm32-unknown-unknown
      - npm ci
      - npm run build:wasm    # rebuild the parser FROM SOURCE (crates/lsdoc-wasm → lsdoc git dep)
      # Call the binary directly — `npx tauri`/npm eats the --flags (validated finding).
      - ./node_modules/.bin/tauri android build --target aarch64 --apk
    output: src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk

AutoUpdateMode: Version
UpdateCheckMode: Tags
CurrentVersion: 0.4.2
CurrentVersionCode: 4002
```

Notes:
- **NO `scandelete`** (validated: the scanner doesn't flag the vendored wasm; listing it
  errors "unused"). Provenance comes from the `npm run build:wasm` step rebuilding it.
- **No `rust-toolchain.toml`** — the recipe's `rustup target add` installs the two targets the
  build needs; a repo-wide pin broke the desktop CI cross-builds (see findings). Point `commit`
  at the latest complete release tag (v0.4.2).
- `npm run build:wasm` needs `wasm-pack` + `wasm-opt` (binaryen); it regenerates
  `lsdoc_wasm_bytes.ts` before `tauri android build` runs `npm run build`, whose
  `check-wasm-pin` then passes (tags match).
- The locally-validated variant differed only by environment (sudo-less: node already on PATH,
  Gradle 8.14.3 provisioned by hand, wasm-pack already installed) and used `commit: <sha>`.

**Reality check:** many Tauri/JS apps stall on step 3 (offline + scanner). Budget weeks of
iteration; the fixes land in the tine repo (toolchain pin, vendoring, deterministic
frontend) and in the recipe (`sudo`, `scandelete`, `output` path), not in F-Droid's court.

---

## Track B — Google Play

More friction than F-Droid for a solo/new account. Start the account + testing gate
EARLY (they run on wall-clock, not effort).

1. **Account:** create a Google Play Console account ($25 one-time). Complete **identity
   verification** (personal account: government ID; can take days). **New personal
   developer accounts must run closed testing with ≥12 testers opted-in for 14
   continuous days before production is unlocked** — start this the moment the account
   exists; it's the long pole.
2. **Build an AAB, enroll in Play App Signing:**
   - Add an `--aab` variant to the `release.yml` android job (`tauri android build
     --aab`; output `app-universal-release.aab`). Keep the APK for GitHub/F-Droid.
   - Enroll in **Play App Signing**: Google holds the app-signing key; we sign the AAB
     with an **upload key** (can reuse the existing `tine-release.jks` as the upload key).
   - Target **API 35** (Play's current minimum for new apps); `minSdk 24` is fine.
3. **Create the app** in Console (package `dev.tine.app`, unique — grab it before anyone):
   - Store listing: title, short + full description, app icon 512×512, feature graphic
     1024×500, ≥2 phone screenshots (reuse/adapt the website shots).
   - **Privacy policy URL** (required) — host a short one on tine.page.
   - **Data safety** form: local-first, no data collected/shared → declare nothing.
   - Content rating questionnaire, target audience, ads = none, government-app = no.
4. **Release rollout:** upload AAB → Internal testing → Closed testing (the 12-tester /
   14-day gate) → Production. Submit for review.

**Cost/timeline:** $25 + identity verification (days) + the 14-day testing gate. The AAB
switch + listing are ~a day of work; the calendar gate is the real wait.

---

## Recommended sequence

1. **A1 self-hosted F-Droid repo** — hours; ships a real, auto-updating F-Droid channel now.
2. **B1 Play account + identity verification + start the 12-tester/14-day closed test** —
   kick off early because it's calendar-bound; do the AAB switch + listing in parallel.
3. **A2 main f-droid.org MR** — the real F-Droid goal; weeks of offline-build iteration.

## Docs to update when shipping

- README + `website/` download section (add F-Droid repo/badge, Play badge when live).
- `CHANGELOG.md` when each channel goes live.
- Remove the shipped item(s) from `docs/BACKLOG.md` Next.
