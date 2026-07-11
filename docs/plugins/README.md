# Tine plugins (experimental API 0.1)

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
2. Give it a lowercase dotted id and edit `manifest.json`.
3. Point its `tine-plugin-sdk` dependency at `plugin-sdk/rust` while developing.
4. Run `cargo build --release`. The template's `.cargo/config.toml` imports a
   host-bounded memory and caps it at 16 MiB.
5. From a Tine checkout, run:

   ```sh
   npm run plugin:check -- /path/to/plugin --json
   ```

6. In Tine, open Settings → Plugins → Choose package and select `manifest.json`
   and the built `.wasm` together. Installation is disabled by default; enable it
   explicitly after reviewing its identity and capabilities.

The checker is designed for agents as well as humans. Its JSON has a stable
`tine-plugin-check/v1` format, precise error codes, the entry SHA-256 digest, exact
imports/exports, declared capabilities, and a coarse risk disposition.

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

API 0.1 effects are notices, focused-block text replacement with an expected-text
precondition, caret insertion, known block decorations, and plugin-local scalar
settings. A write effect requires `graph.write.block`, may target only the block Tine
included in the triggering event, records normal undo, and reaches disk only through
Tine's existing conflict-checked persistence engine.

## Platforms

`platforms` may contain `desktop`, `android`, and `ios`. Omitting it means desktop
only. Each contribution may narrow the manifest-level list. The API contains no
desktop process primitive, so the same guest can run on Android; Tine may ship a
platform's installation UI later than its runtime support.

## Compatibility and versioning

The manifest identifies plugin API `0.1`; protocol messages use
`protocolVersion: 1`. Tine refuses mismatches. During 0.x, a breaking host change
must bump the plugin API, show a clear incompatible state, and leave the old package
disabled and intact. Published package versions are immutable and addressed by
`id`, SemVer version, and SHA-256.

See [the threat model](threat-model.md), [registry policy](registry-policy.md), and
[porting guide](porting-logseq-obsidian.md).
