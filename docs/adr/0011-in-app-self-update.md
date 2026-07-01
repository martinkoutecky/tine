# 0011. In-app self-update on Windows/Linux via the Tauri v2 updater

- **Status:** Accepted
- **Date:** 2026-07-01

## Context

Tine already tells a user when a newer release exists (`src/update.ts`: a startup
GitHub-releases check → a sticky toast). The toast's "Download" only opened the
releases page — the user still downloaded and reinstalled by hand.

Tauri v2 ships a first-party self-updater (`tauri-plugin-updater`): the bundler
emits per-target update artifacts + a minisign `.sig`, the app checks a static
`latest.json`, verifies the download against a compiled-in public key, installs in
place, and relaunches. The forces that make this a real decision, not a free win:

- It introduces a **new, permanent root of trust** — a minisign keypair, entirely
  separate from OS code-signing. If the private key leaks it must be rotated, and
  installs pinned to the old public key can no longer auto-update (manual reinstall).
- **Per-platform install reality differs.** AppImage self-updates cleanly (pure
  file ops, no root) *only* when launched as an AppImage on a writable mount.
  Windows NSIS runs the installer (a brief passive UI; a UAC prompt if per-machine).
  macOS **requires a code-signed + notarized** bundle — replacing an unsigned app
  re-triggers Gatekeeper quarantine. Tine's macOS/Windows builds are currently
  **unsigned** (empty `APPLE_*` secrets), so macOS self-update is a non-starter today.

## Decision

We will ship the Tauri updater for **Windows and Linux (AppImage)** and keep
**macOS on the manual path** until a paid Apple Developer ID + notarization exist.

- `tauri.conf.json`: `bundle.createUpdaterArtifacts: true` and a `plugins.updater`
  block (GitHub `releases/latest/download/latest.json` endpoint, the minisign
  `pubkey`, Windows `installMode: "passive"`).
- Rust: register `tauri-plugin-updater` + `tauri-plugin-process`; grant
  `updater:default` + `process:allow-restart` in the default capability.
- Frontend: the existing GitHub-releases toast stays the cross-platform **notifier**.
  Its "Download" action self-updates only where it's safe — packaged Tauri app,
  non-macOS: `check()` → `downloadAndInstall()` → `relaunch()`. macOS, the browser
  mock, and **any** failure (no `latest.json` yet, bad signature, offline) fall back
  to opening the releases page. It can never brick the app.
- CI: `tauri-action` gets `TAURI_SIGNING_PRIVATE_KEY(+_PASSWORD)` and
  `updaterJsonPreferNsis: true`; it auto-generates and uploads `latest.json` + the
  `.sig` files. The `APPLE_*` signing story is unchanged (still deliberately unset).

## Consequences

- Windows/Linux users update in one click instead of a manual reinstall.
- We now **own a minisign keypair forever**: it must be generated once
  (`tauri signer generate`), the public key pasted into `plugins.updater.pubkey`,
  the private key + password stored as the two CI secrets, and the key backed up and
  guarded. **Both must be in place before the next release tag** — with
  `createUpdaterArtifacts: true` the bundler signs updater artifacts, so a tagged
  release without the signing secret fails, and a release whose embedded `pubkey`
  doesn't match the signing key ships clients that can't verify updates. (This is
  Martin's step; the repo carries a placeholder pubkey until then.)
- macOS is explicitly excluded and stays on the download-page fallback; revisit when
  Apple signing is wired in (would also unblock notarized macOS builds generally).
- AppImage self-update only works when Tine is *run as* the AppImage on a writable
  mount — not for a raw binary (e.g. Martin's Syncthing dev binary at
  `~/research/tine`), which keeps the manual path.
