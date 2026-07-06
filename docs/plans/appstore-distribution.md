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

### A2 — Main f-droid.org repository (the real goal; slow, iterative)

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
- **Rust toolchain.** Pin it: add `rust-toolchain.toml` at repo root
  (`[toolchain] channel = "1.xx"` + `targets = ["aarch64-linux-android", …]`). The recipe
  installs rustup in `sudo` if the buildserver lacks the pinned toolchain.

**Steps:**

1. **Repo prep (in tine, land before the MR):**
   - Add `rust-toolchain.toml` pinning the channel + android targets.
   - Do a **network-off** end-to-end build locally to prove it's offline-buildable:
     `npm ci` then `npx tauri android build --apk` with the box offline (netns / unplug).
     If cargo/npm need the network, pre-fetch: `cargo fetch` (Cargo.lock is committed) and
     `npm ci` while online, then re-run offline. Vendor cargo (`cargo vendor` +
     `.cargo/config.toml`) only if F-Droid's env can't fetch crates.
   - Note the **exact output APK path** the build produces (expected
     `src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk`
     — verify, since the flavor/name comes from the generated `tauri.build.gradle.kts`).
2. **Fork `gitlab.com/fdroid/fdroiddata`**; add `metadata/dev.tine.app.yml` (starter
   below). Use `Builds:` with a freeform `build:` that drives the Tauri CLI + `output:`.
3. **Validate locally with the real F-Droid buildserver BEFORE the MR** — this is where
   the weeks go. Either the Docker image
   `registry.gitlab.com/fdroid/fdroidserver:buildserver` or `fdroid build -l dev.tine.app`
   from an `fdroidserver` checkout. Iterate: build error → fix recipe/repo → repeat. Then
   run `fdroid scanner dev.tine.app` and `fdroid lint dev.tine.app` clean.
4. **Open the MR** to fdroiddata, respond to the reviewer. After merge the first build can
   take days–weeks; then `UpdateCheckMode: Tags` auto-proposes each new `v*` tag.
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
  - versionName: 0.4.0
    versionCode: 4000
    commit: v0.4.0
    sudo:
      - apt-get update || true
      - apt-get install -y nodejs npm
      # If the pinned Rust toolchain isn't preinstalled, add rustup here.
    ndk: 26.3.11579264
    build:
      - npm ci
      - npx tauri android build --apk --target aarch64
    output: src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk

AutoUpdateMode: Version
UpdateCheckMode: Tags
CurrentVersion: 0.4.0
CurrentVersionCode: 4000
```

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
