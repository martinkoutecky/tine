# Tine plugins (experimental API 0.2)

Tine plugins are small WebAssembly guests. They receive versioned JSON events and
return inert effects which Tine validates and applies. They do **not** run JavaScript
inside Tine, receive the frontend store, or access the DOM, Tauri, files, processes,
the network, or arbitrary graph paths.

API 0.x is intentionally experimental: Tine may reject an incompatible plugin after
an update rather than preserve a dangerous or ill-shaped interface. Disabling a
plugin must leave every graph readable; plugins may not invent a required graph
storage format.

## Build the first plugin

1. Copy `plugin-sdk/templates/rust/` to a new public repository.
2. Give it a lowercase dotted id, edit `manifest.json`, and replace the sample
   repository and author metadata.
3. Run `cargo build --release`. The template pins a reviewed Tine SDK revision;
   update that revision deliberately when adopting a newer API. Its
   `.cargo/config.toml` imports a
   host-bounded memory and caps it at 16 MiB.
4. From a Tine checkout, run:

   ```sh
   npm run plugin:check -- /path/to/plugin --json
   ```

5. In Tine, open Settings → Plugins → Choose package and select `manifest.json`
   and the built `.wasm` together. Installation is disabled by default; enable it
   explicitly after reviewing its identity and capabilities.

The checker is designed for agents as well as humans. Its JSON has a stable
`tine-plugin-check/v1` format, precise error codes, the entry SHA-256 digest, exact
imports/exports, declared capabilities, and a coarse risk disposition.
If `port-gap.json` is present, the same command validates its structured explanation
of omitted behavior and includes `portGap.status` in the report.

## ABI

The module must import exactly one item:

```text
env.memory : WebAssembly.Memory
```

It exports three functions using 32-bit integers:

```text
tine_alloc(length) -> pointer
tine_handle(pointer, length) -> result_pointer
tine_result_len() -> result_length
```

The input and output are UTF-8 JSON, each capped at 256 KiB. Tine creates one
Web Worker per plugin, gives it at most 16 MiB by default, and terminates the whole
worker when an invocation exceeds 250 ms. The worker imports no host functions, so
there is no hidden clock, random source, logging channel, WASI, or browser authority.

## Contribution points

- `commands`: command-palette entries. A command receives only the focused block
  snapshot when one exists.
- `slashCommands`: editor slash-menu entries. A guest can return `insert-at-caret`;
  Tine discards the result if the block changed while the guest ran.
- `blockDecorations`: a small host-owned visual vocabulary (`thread-lines`,
  `badge`). A plugin cannot inject HTML or CSS.

API 0.2 effects are notices, focused-block text replacement with an expected-text
precondition, caret insertion, known block decorations, and plugin-local scalar
settings. A write effect requires `graph.write.block`, may target only the block Tine
included in the triggering event, records normal undo, and reaches disk only through
Tine's existing conflict-checked persistence engine.

## Platforms

`platforms` may contain `desktop`, `android`, and `ios`. Omitting it means desktop
only, and the starter template declares only desktop. Add Android or iOS only after
testing the package on that platform. Each contribution may narrow the
manifest-level list. The API contains no desktop process primitive, so a conforming
guest can run unchanged on mobile, but a platform declaration is a compatibility
claim rather than an aspiration.

Tine 0.6 exposes the complete plugin/theme lifecycle on Android. `ios` is reserved
for portable packages but Tine 0.6 does not ship an iOS host. See the
[Android phone smoke test](android-smoke.md) for the hardware release gate.

## Compatibility and versioning

The manifest identifies plugin API `0.2`; protocol messages use
`protocolVersion: 2`. Tine refuses mismatches. During 0.x, a breaking host change
must bump the plugin API, show a clear incompatible state, and leave the old package
disabled and intact. Published package versions are immutable and addressed by
`id`, SemVer version, and SHA-256.

## Publish a community plugin

Keep the plugin source public, choose an OSI-approved license, commit the built Wasm
artifact, and add screenshots showing the behavior in Tine. Then open a submission
against the
[Tine plugin registry](https://github.com/martinkoutecky/tine-plugin-registry)
using its schema-v2 submission template. Registry automation verifies the manifest,
Wasm imports, digest, source revision, license, provenance, screenshots, and declared
capabilities. Passing low-risk packages may publish automatically; uncertain or
elevated results are quarantined for review. A published version is immutable, so
fixes require a new SemVer version.

See [the threat model](threat-model.md), [registry policy](registry-policy.md),
[porting guide](porting-logseq-obsidian.md), and [Android smoke test](android-smoke.md).
