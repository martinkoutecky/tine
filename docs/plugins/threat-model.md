# Plugin threat model

The plugin platform assumes source, package bytes, metadata, issue text, and audit
prompts are hostile. “Martin published it,” “an AI wrote it,” and “the source looks
small” are not security boundaries.

## Protected assets

- graph text, assets, configuration, backups, and physical paths;
- operating-system files, credentials, processes, clipboard, and network identity;
- Tine's privileged Tauri commands and frontend store;
- the registry publisher credential and the local auditor's ChatGPT/Codex session;
- availability: a plugin may not freeze or exhaust the app.

## Runtime boundary

Ordinary plugins are WebAssembly modules in dedicated Web Workers. The loader rejects
every import except bounded `env.memory`; it does not provide WASI, browser, Tauri,
network, filesystem, process, environment, time, randomness, dynamic linking, or DOM
functions. Input/output and effect counts are capped. A deadline terminates the worker
rather than attempting cooperative cancellation.

Guest output is parsed as strict JSON. Unknown effects and enum values are rejected.
Effects are checked against the manifest capability, event kind, contribution, input
object set, and—on mutation—the block's expected prior text. The host owns all UI and
all graph writes. No plugin-originated write bypasses store undo or persistence's
base-revision, graph-generation, tombstone, conflict, and serialization guards.

This contains ordinary plugin bugs and malicious guest code. It does not make an
approved host contribution safe by magic: every new capability is a security API and
requires its own abuse analysis, bounded inputs/outputs, user disclosure, and tests.
Network, process, arbitrary filesystem, custom DOM, and synchronization capabilities
are explicitly deferred.

## Package and update boundary

The Rust backend revalidates id/version path components, package size, JSON, and WASM
magic before writing. Each `id@version` is immutable. Installation is atomic and
disabled; enabling separately re-parses the manifest, checks platform and revocation,
verifies the on-disk SHA-256, instantiates the bounded runtime, and activates it. A
trap, timeout, invalid response, digest mismatch, incompatibility, or revocation
disables the plugin without blocking graph startup.

The registry signs/indexes digests rather than mutable download URLs. A catalogue
entry never grants more authority than the installed manifest displays. Revocation
is per immutable version, reasoned, timestamped, and signature-verified by Tine.

## Local audit boundary

Hostile plugin builds run in a fresh rootless Podman container with no host home,
secrets, GitHub write credential, or Codex authentication; network is off after any
pinned dependency-fetch phase. The AI reviewer is a separate process which reads
source and deterministic artifacts but never executes plugin code. Its subscription
credential is not mounted into the build container. A third narrow publisher process
can post only the structured report/status for the submission it leased.

AI review is advisory defense in depth, not the runtime sandbox. Prompt injection can
mislead a review, so deterministic authority/import/schema/digest checks decide the
minimum gate and uncertainty quarantines rather than publishes.

## Known residual risks

- engine or WebView vulnerabilities in WebAssembly/Worker isolation;
- CPU denial within the deadline and aggregate load from many enabled plugins;
- socially misleading labels/notices despite bounded inert rendering;
- a trusted host capability with an unforeseen confused-deputy path;
- compromise of the Tine release key, registry signing key, or local publisher.

Mitigations include conservative capabilities, per-plugin termination, aggregate
limits, transparent reports, immutable provenance, signed revocation, and keeping
privileged plugin tiers out of API 0.1.
