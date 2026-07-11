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
Verify the generated registry archives and git pins with:

```bash
node scripts/check-flatpak-cargo-sources.mjs
```

The full no-network build runs in `.github/workflows/flatpak.yml`. It uses a
privileged Flatpak builder container because ordinary development sandboxes
generally cannot nest bubblewrap or mount `/proc`.
