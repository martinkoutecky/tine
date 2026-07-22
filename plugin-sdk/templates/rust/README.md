# My Tine plugin

This is the standalone Rust starter for a capability-limited Tine WebAssembly
plugin. It targets plugin API 0.2 and defaults to desktop support until you have
tested the package on another platform.

1. Replace the package name, dotted plugin id, author, source URL, and description.
2. Implement event handling in `src/lib.rs` using only the declared capabilities.
3. Build with `cargo build --release`.
4. From a Tine checkout, run
   `npm run plugin:check -- /path/to/this/repository --json`.
5. Select `manifest.json` and the built Wasm together in Settings → Plugins.

The SDK dependency is pinned to a reviewed Tine commit. Update the pin only when you
intend to adopt a newer plugin API, then rebuild and rerun the checker. Before
publishing, add screenshots captured in Tine and follow the submission instructions
in the [Tine plugin registry](https://github.com/martinkoutecky/tine-plugin-registry).
