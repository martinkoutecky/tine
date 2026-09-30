# App identity and the master → og transition

Tine's app identifier (`page.tine.Tine` for the released app) keys everything a
user's install owns outside their graph. That covers the app-data dir
(`~/.local/share/<id>` on Linux), the config dir (`~/.config/<id>`, window
geometry), the WebKitGTK localStorage inside the app-data dir, the Linux desktop
entry and Wayland `app_id`, the single-instance lock, and on Android the
application id. Master ships stable **Tine**; og ships **Tine Beta**, a separate
app with its own settings and a `beta` updater channel. Stable never sees Beta
updates, and Beta never installs stable releases (both notifier and installer
validate through `checkedBetaUpdate` in `src/update.ts`); Beta publication is a
prerelease and never becomes GitHub's `latest` release.

The internal switch key remains `experiment`. Its Beta identity is new: the
former experiment identity never shipped, so there is no migration from it.
Desktop seeding still reads only stable Tine's compatible settings and leaves
stable data intact. The local deploy destination remains `~/research/tine-og`.
Beta release versions use `X.Y.0-beta.N` (N = 1–999), with the Android code
mapping and packaging checks in `scripts/release-policy.mjs`; numeric versions
remain accepted for the current campaign build and stable tooling.

## The switch

`src-tauri/app-identity.json` is the only place either identity is written:

```json
{ "ship": "experiment",
  "identities": {
    "release":    { "identifier": "page.tine.Tine",   "productName": "Tine",    "androidApplicationId": "page.tine.app", "deployName": "tine" },
    "experiment": { "identifier": "page.tine.TineBeta", "productName": "Tine Beta", "androidApplicationId": "page.tine.beta",  "deployName": "tine-og" } } }
```

To flip it, run `node scripts/set-app-identity.mjs release` (or
`experiment`). The script rewrites the switch and the files derived from it:

| Derived place | How |
|---|---|
| `src-tauri/tauri.conf.json` `identifier`, `productName`, main window `title` | rewritten by the script; `src-tauri/build.rs` refuses to compile when they disagree with the switch |
| Rust code (`app_identity.rs`, `linux_window_identity.rs`, the seed) | `build.rs` emits `TINE_APP_IDENTIFIER`, `TINE_PRODUCT_NAME` and `TINE_RELEASE_IDENTIFIER` |
| Release filenames (`scripts/release-layout.mjs`) | derive from `productName`; whitespace becomes `-` for GitHub assets, while `sourceAssets` names Tauri outputs verbatim |
| Android `applicationId` (`src-tauri/gen/android/app/build.gradle.kts`) | rewritten by the script; the Kotlin `namespace` stays `page.tine.app` |
| Deploy destination (`scripts/deploy.sh`) | `~/research/<deployName>` |
| Native E2E journeys | `scripts/lib/app-identity.mjs` (`APP_ID`, `IDENTITY`) |
| Flatpak (`.github/workflows/flatpak.yml`) | refuses to build unless `ship` is `release`; the manifest id is the release identifier |

Android release preparation (`scripts/release-workflow-inputs.mjs android-config`)
uses the committed Gradle Kotlin namespace for Tauri's Java source lookup in the
ephemeral runner checkout. It keeps the switch-derived Gradle `applicationId`.
`build.rs` accepts that namespace only on Android and verifies `applicationId`
against the switch. Direct Android `cargo check` keeps the canonical desktop
config; desktop builds require the selected desktop identifier.
The Android version code uses `releaseVersion`, including the Beta sequence.

Release staging copies signed bundles without changing their bytes. Only zsync
headers (`Filename` and `URL`) are changed to the canonical published AppImage
name, preserving the binary checksum payload. The workflow's embedded AppImage
update pattern, Windows portable lookup, APK name, updater manifest and publisher
all consume the same layout. Beta updates remain restricted to the `beta` release.
`scripts/test-release-identity.mjs` drives staging and assembly for both ships.

A few identity-bearing places need no file of their own:

- The iOS bundle id is Tauri's `identifier`. No `gen/apple` project is
  checked in.
- The Linux window class, Wayland `app_id` and `.desktop` file come from
  `TINE_APP_IDENTIFIER`.
- The Cargo binary is `tine` in both settings; only the deploy name differs.

A desktop keyboard shortcut or dock pin bound to `page.tine.Tine.desktop` does
not apply to a Beta build, because its window reports
`page.tine.TineBeta`. It applies again once the switch ships `release`.

`node scripts/set-app-identity.mjs --check` exits 1 if any derived file has
drifted. `src/appIdentity.guard.test.ts` enforces the switch. It checks that
every derived file matches it, that both settings round-trip, and that the
release identity is the one master ships. It also checks that no source file
outside the derived set and this front door spells either identifier or
"Tine Beta".

## What the released identity finds in a master user's dir

These are the entries master writes in its app-data dir, and how og treats
each one. The inventory was taken from master `6c380173` and checked on a dir
written by master's own binary (`scripts/og-identity-transition.mjs`). None is
rewritten into another format, and none is deleted or made into an error.

| Entry | Class | Notes |
|---|---|---|
| `tine-settings.json` | read as-is | Same file and keys. og preserves keys it does not know (`link_autocomplete_policy`, theme composition, …) when it saves. `last_graph_path` / `known_graphs` open master's graph. |
| `sessions/<graph>-<fnv>.json`, `…-workspaces.json` | read as-is | Same FNV naming and v1 workspace validator: tabs and last page come back. |
| `sessions/<graph>-<fnv>-notices.json` | read/write as-is | Query-crossing dismissals are device-local and keyed by graph, including live graph switches. The old global OG boolean is ignored. Unknown dismissal keys and fields survive writes. |
| `plugins/<id>/<version>/` | read as-is | Same package layout. |
| WebKit localStorage (`localstorage/`, `storage/`) | read as-is | Same origin and keys (`tine.graphPath`, theme, shortcuts, sidebar, recents). |
| `.window-state.json` (config dir) | read as-is | tauri-plugin-window-state. |
| `diagnostics/process.lock`, `session-active` | read as-is | Same semantics. The single-instance lock stops master and og from running at once. |
| `diagnostics/*.jsonl` | disjoint | og writes `history.jsonl`; master's `current`/`previous` files are left alone. |
| `backups/<graph>/<stamp>/` schema 3 | read as-is, never pruned | Since og-B (ADR 0062) og writes the same schema-3 layout and lists and restores master's snapshots. og marks its own with `"writer": "og"` (master ignores the field) and its keep-count prunes only its own and schema-2 ones: `backup::is_foreign_snapshot`. Master's keep-count counts every snapshot, og's included, as it already did for og's schema-2 ones. |
| `backups/…/.partial-*` | cleaned | A crashed, never-published snapshot. It is cleaned by whichever Tine runs, and the single-instance lock means it is never a live one. |
| `direct-files-projections/`, `direct-move-recovery/`, `conflict-capsules/`, `mediakeys/`, `hsts-storage.sqlite`, `WebKitCache/` | master-only | og has no reader and never opens them. They are byte-identical after an og run. |
| `concord-ledger/<root>/` | **conflict (open)** | Same path and schema number, but a different layout. Each build's prune deletes the other's pin files. See Open items. |

Inside the graph dir, master writes these `.tine*` entries, and og treats them
as follows:

- `logseq/.tine-trash/` and `assets/.tine-restore-recovery/` are shared.
  og writes and reads the same layouts.
- `.tine-sync/` (ex-Managed Storage, ADR 0066) is read by neither build and
  left alone.
- The write-probe sentinels (`.tine-capability-*`, `.tine-write-probe-*`) are
  transient.

Opening a graph writes nothing into it on either build, and the differential
checks that the whole graph dir is byte-identical.

Rollback (C) means master opens a dir og has used with the user's config
intact. It holds because og writes only the shared formats above, in the same
layout master reads.

## Beta config seed

While the Beta identity ships, its app-data dir starts empty and a tester
would see Welcome instead of their graphs. `src-tauri/src/experiment_config_seed.rs`
fixes that, once, before the webview exists. It runs only when the experiment
dir has no configured graph and the released dir has one. It then copies the
allowlist (`tine-settings.json`, `sessions/`, `plugins/`, `localstorage/`,
`storage/`, plus `.window-state.json` if the experiment has none) into a
staging dir, fsyncs it, and renames it into place.

- The released dir is only read. The index, projections, backups, ledger and
  anything unknown are never copied.
- In-scope scenarios:
  - Crash or power loss mid-copy leaves only the staging dir, which the next
    launch discards and rebuilds.
  - A disk error abandons the seed, and the build starts on Welcome. This is
    recovery, not a refusal.
  - An experiment dir that only ever showed Welcome is renamed to
    `<id>.pre-seed.N`, not deleted.
- Config and external browser stores publish independently. If startup stops after
  config publication, the next launch retries any missing external browser store
  without replacing existing experiment browser state. Windows uses the native
  LocalData `EBWebView` directory; macOS uses `Library/WebKit/<id>/WebsiteData`.
  Fixture copy/retry tests prove preservation; native browser reopening remains
  a Windows/macOS integration gate.
- In a release build it is a no-op (`APP_IDENTIFIER == RELEASE_IDENTIFIER`).
- Only on desktop. Mobile app data is private to each application id.

**Delete it when the switch ships `release`.** Remove the file, its `_tests.rs`,
the `mod` line and the one call in `lib::run()`, and its
`APPROVED_WRITER_SITES` entry in `scripts/lib/og-enforcement.mjs`.
`src/appIdentity.guard.test.ts` fails until you do.

About, update notifications and copied version information derive their app name
from the same switch through `src/appIdentity.ts`. **Copy version** preserves
the complete runtime version, including the Beta sequence.

## Proof

`scripts/og-identity-transition.mjs` (Linux, tauri-driver + Xvfb) runs a
released Tine binary on a copy of a test graph in an isolated HOME: the user
opens a page and toggles the theme. It then checks three things:

- An experiment og build started with no graph argument shows the same graph,
  page and theme, with no Welcome. Master's dir stays byte-identical, and no
  master-only artifact is copied.
- A release-identity og build shows the same config and changes no master-only
  artifact or master snapshot.
- Master reopens the dir og used with the same config.

On the pre-change og build the first check fails (Welcome, default theme).

## Open items

- **Concord ledger namespace.** og and master share `concord-ledger/<root>/`
  with incompatible layouts, and each prune deletes the other's pins. The
  ledger is disposable (a lost pin costs a later conflict prompt, not data).
  Still, og should namespace its dir before the flip. The owner is
  `concord_ledger.rs` (lane 20a).
- **Legacy identifiers are restored.** Before settings or Tauri starts, a release
  build migrates the newest `page.tine.app` / `dev.tine.app` app-data directory
  into the switch-derived released directory. Beta builds do not run it.
  A destination containing user state (including backups) prevents migration.
  A Welcome-only scaffold is parked and retained; complete payload directories
  move without translating their contents. Copy fallback preserves its source.
  The native one-shot flag produces a sticky startup toast explaining the move
  and that some app preferences may need setting again.
- **Unwritable data home is restored.** `data_home.rs` relocates app data to
  `~/.tine-data` for that launch when the usual location is unwritable, and
  App shows the sticky notice with that location.
- Windows/macOS external browser-store seeding is implemented and retries after
  interrupted publication. Actual WebView2/WKWebsiteDataStore reopening and
  preference continuity still require native platform proof.
