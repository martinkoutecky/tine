# Plugin revocation native-proof fixture

This directory reserves `page.tine.e2e.revocation-sentinel@0.0.1` exclusively
for Tine's native revocation journey. It must never be submitted to or published
by the community registry. Its only visible contribution is Tine's host-owned
`thread-lines` decoration; its WebAssembly guest returns no effects and cannot
read or write a graph.

## Public provenance

- `control-index.json` and `control-index.json.sig` are the production-key-signed
  empty index published in Tine commit
  `faf03b98392f39f362e89db4db3e8cc7bda79da4` (`src-tauri/src/plugins.rs`,
  `registry_public_key_has_the_expected_identity`). They are a positive control:
  the persisted enabled sentinel must visibly activate when this verified cache
  contains no revocation.
- `registry-ed25519.pub.pem` is the matching public key from the public
  `martinkoutecky/tine-plugin-registry` repository (key-rotation commit
  `2a723b5a74890cc7cf5fb507c4e97ef90712dd59`). It contains no secret material.
- `revoked-index.json` is canonical JSON in the same sorted-key serialization
  used by `tine-plugin-registry/auditor/publisher.py`. It names only the reserved
  sentinel identity.
- `sentinel-src/` is the complete harmless guest source. It implements only the
  three stable host ABI exports and returns an empty-effects response. It is
  deliberately dependency-free: a path dependency on the SDK made Rust's
  crate disambiguation and linked function order depend on the checkout's
  absolute path even though the source and toolchain were locked. Rebuild it with
  `source scripts/env.sh && TINE_PLUGIN_OFFLINE=1 npm run plugin:revocation-fixture:build`.

Verify both the ABI and byte-for-byte reproducibility from two distinct absolute
source roots with `npm run plugin:revocation-fixture:repro`. Before this change,
the type/import/function/table/global/export/start/data sections matched between
worktrees, but the element and code sections did not; there were no custom
sections to strip safely. Removing the irrelevant path dependency makes every
section identical without rewriting compiled WebAssembly.

The committed byte identities are recorded in `fixture.json`. The sentinel WASM
is 12,989 bytes and has SHA-256
`be8035e6e648b3b7e3bb09e1299926ad8213656471701256fb19ee0bec7fad56`.

## Completed one-signature handoff

`revoked-index.json.sig` is the independently verified offline production
Ed25519 signature for `revoked-index.json`. Its committed byte identity is
recorded as `revokedSignatureSha256` in `fixture.json`.

The fixture checker verifies the signature under the committed production public
key and confirms that `plugin-revocation` is registered in `linux-release`:

```bash
npm run plugin:revocation-fixture:check
```

The remaining work is to run the registered native revocation journey and the
manager-owned unchanged-binary burn-in. Until those steps are complete, this
fixture is not release evidence.
