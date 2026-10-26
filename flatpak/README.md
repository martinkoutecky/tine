# Flatpak offline sources

Flathub builds without network access. `cargo-sources.json` and
`node-sources.json` therefore materialize every dependency referenced by the
lockfiles.

After changing `package.json` or `package-lock.json`, regenerate with the
current `flatpak-node-generator` from
[`flatpak-builder-tools`](https://github.com/flatpak/flatpak-builder-tools):

```bash
flatpak-node-generator -o /tmp/node-sources.json npm package-lock.json
```

The project sets `PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1`; remove generated source
objects whose URL begins with `https://cdn.playwright.dev/`, then replace
`flatpak/node-sources.json`. Do not remove the small `INSTALLATION_COMPLETE`
inline markers. Verify the result with:

```bash
node scripts/check-flatpak-node-sources.mjs
```

After changing Rust dependencies, regenerate `cargo-sources.json` with
`flatpak-builder-tools/cargo/flatpak-cargo-generator.py` from `Cargo.lock`.
Then separate git packages from registry crates so equal name/version pairs do
not collide inside Cargo's offline directory source:

```bash
node scripts/normalize-flatpak-cargo-sources.mjs
```

Verify the generated registry archives and git pins with:

```bash
node scripts/check-flatpak-cargo-sources.mjs
```

The full no-network build runs in `.github/workflows/flatpak.yml`. It uses a
privileged Flatpak builder container because ordinary development sandboxes
generally cannot nest bubblewrap or mount `/proc`.

## Identity and the Beta bundle

The files here carry the released identity. For any other identity
(`src-tauri/app-identity.json` `ship`), `scripts/derive-flatpak-identity.mjs`
derives the manifest, `.desktop` and metainfo at build time into untracked files
(`flatpak/<id>.ci.yml`, `flatpak/derived/`). The Beta bundle therefore installs
beside a stable Tine Flatpak. The identity requested must match the one the tree
ships; a mismatch is refused. See `docs/app-identity.md`.

## Tray library

`shared-modules/` is a byte-for-byte copy of
[flathub/shared-modules](https://github.com/flathub/shared-modules) at
`cb9ec602a1ece1c76d5a4f8aa1d87c4a6bf99c3e` (`libayatana-appindicator/` and the
`intltool/` module it includes). It builds libayatana-appindicator3, libdbusmenu,
ayatana-ido and libayatana-indicator so the system tray (GH #625) works in the
sandbox. Every source in it is pinned by commit or sha256 and fetched during
flatpak-builder's download phase like the cargo and node sources; the build
itself stays offline. To update, copy the directories from a newer commit of that
repository and rerun the Flatpak build.
