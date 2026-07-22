# Plugin platform security audit — 2026-07-12

Scope: the experimental plugin and theme platform on branch `plugins`, its three
launch plugins, and the signed community registry/auditor. This is a pre-release
audit, not a claim that automated review can guarantee safety.

## Results

- `npm audit`: 0 advisories across 486 production, development, optional, and peer
  dependencies.
- RustSec: 0 unignored vulnerability advisories in every launch-plugin and SDK
  lockfile. The application lockfile is also clear after updating `anyhow` to
  1.0.103 and the runtime `plist` path to 1.10.0 / `quick-xml` 0.41.0.
- Signed revocation was exercised for both executable plugins and inert themes.
  A revoked theme now cannot be installed or selected, and an active revoked theme
  falls back to Default while remaining uninstallable.
- Registry index/signature/submission validation passes for all eight immutable
  versions. The local auditor's 19 security/configuration tests pass.
- Hostile plugin builds fail closed into rootless Podman or the bounded Bubblewrap
  fallback. Neither path exposes the host home, publisher key, GitHub credential,
  Codex authentication, or network during submitted build-script execution.

## Narrow RustSec exceptions

The application audit command explicitly ignores `RUSTSEC-2026-0194` and
`RUSTSEC-2026-0195` for one build-only dependency path:

```text
wayland-scanner 0.31.10 (proc macro) -> quick-xml 0.39.4
```

The proc macro parses fixed Wayland protocol XML shipped by its dependency while
Tine is compiled. It is absent from the runtime path and cannot parse a graph,
plugin, registry response, network response, or other user-controlled XML. Upstream
commit `d07c4f91f28b42e5a485823ffd9d8d5a210b1053` moves the scanner to `quick-xml`
0.41, but pinning the whole unreleased upstream workspace is incompatible with the
released Wayland client crates. Remove these two audit exceptions when a compatible
`wayland-scanner` release reaches crates.io.

RustSec also reports informational warnings for the transitive GTK3 stack used by
Tauri on Linux. In particular, `RUSTSEC-2024-0429` concerns
`glib::VariantStrIter`; Tine does not call that API. GTK3's unmaintained notices and
this function-specific warning remain inherited platform risk until Tauri/WebKit's
Linux stack moves off those bindings.

## Commands

```sh
npm audit --json
cargo audit --file Cargo.lock --ignore RUSTSEC-2026-0194 --ignore RUSTSEC-2026-0195
cargo audit --file plugin-sdk/rust/Cargo.lock
cargo audit --file plugin-sdk/templates/rust/Cargo.lock
cargo audit --file community-plugins/bullet-threading/Cargo.lock
cargo audit --file community-plugins/query-filter/Cargo.lock
cargo audit --file community-plugins/heading-level-shortcuts/Cargo.lock
```

The normal TypeScript, frontend, Rust, package-checker, mobile-layout, Linux release
E2E, and side-by-side deployment gates are recorded separately in the release
handoff; this document records the security-specific review and residual risks.
