# Tine's Flatpak bundle

Each release publishes our own installable `.flatpak` bundle, built by
`.github/workflows/flatpak.yml` from the checked-out tree. FlatPark separately
repackages the `.deb`; it does not build this bundle.

The Tine module fetches dependencies during the build using `npm ci` and
`cargo build --locked`. Both use the checked-in lockfiles. Network access is
enabled with `build-options.build-args: [--share=network]`, as documented in
the [flatpak-builder reference](https://docs.flatpak.org/en/latest/flatpak-builder-command-reference.html#build-options).
The privileged hosted builder can nest bubblewrap and mount `/proc`, which
ordinary development sandboxes generally cannot do.

Before building, a separate job runs the shared quick-share viewer readiness
check. Identity derivation carries its output into the manifest's build
environment. A failed viewer check leaves quick share disabled, as on the
other release platforms; see `docs/contracts/quick-share.md`.

## Identity and the Beta bundle

The files here carry the released identity. For any other identity
(`src-tauri/app-identity.json` `ship`), `scripts/derive-flatpak-identity.mjs`
derives the manifest, `.desktop` and metainfo at build time into untracked files
(`flatpak/<id>.ci.yml`, `flatpak/derived/`). The Beta bundle therefore installs
beside a stable Tine Flatpak. The identity requested must match the one the tree
ships; a mismatch is refused. See `docs/app-identity.md`.

## Tray library

`shared-modules/` is a byte-for-byte copy of
[the upstream shared modules](https://github.com/flathub/shared-modules) at
`cb9ec602a1ece1c76d5a4f8aa1d87c4a6bf99c3e` (`libayatana-appindicator/` and the
`intltool/` module it includes). It builds libayatana-appindicator3, libdbusmenu,
ayatana-ido and libayatana-indicator so the system tray (GH #625) works in the
sandbox. Every source in it is pinned by commit or sha256 and fetched during
flatpak-builder's download phase. To update, copy the directories from a newer commit of that
repository and rerun the Flatpak build.
