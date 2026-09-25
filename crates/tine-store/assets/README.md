# Vendored export assets

Static assets bundled into the published HTML export by `src/publish.rs`
(`include_str!`-ed and written into the `publish/` output directory).

## `fuse.min.js`

- **Fuse.js v6.4.6** — lightweight fuzzy-search, UMD build (exposes `window.Fuse`).
- Source: <https://cdn.jsdelivr.net/npm/fuse.js@6.4.6/dist/fuse.min.js>
- License: **Apache-2.0** © 2021 Kiro Risk (license header preserved in the file).
- Why this version: it matches the Fuse.js version original Logseq ships in its
  published graphs (`og/package.json` → `"fuse.js": "6.4.6"`), so Tine's exported
  search behaves the same. The export configures it to mirror OG's block-search
  params (threshold 0.35, block-level content). Bundling it (rather than a CDN
  link) keeps the exported site fully functional offline / over `file://`.
