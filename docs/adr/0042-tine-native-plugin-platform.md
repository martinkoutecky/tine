# 0042. Tine-native plugins run as capability-limited WebAssembly guests

- **Status:** Accepted
- **Date:** 2026-07-11

## Context

AI-assisted development makes small integrations cheap enough that many users can
build or port them, while accepting the same code into Tine core would make Martin
responsible for its security, product fit, and permanent maintenance. A plugin
platform can separate “useful now” from “part of Tine forever,” but plugins operate
next to real graphs, so an ordinary JavaScript extension loaded into Tine's Tauri
webview would inherit far too much authority. An iframe is not a reliable privilege
boundary on every Tauri target. Literal Logseq or Obsidian compatibility would also
import their APIs and architectural commitments; an AI agent can instead port a
plugin to a small Tine-native API.

## Decision

- Tine will provide its own, initially experimental plugin format. A plugin package
  contains a manifest and a WebAssembly module whose only permitted import is a
  bounded, host-provided memory. Browser, DOM, Tauri, WASI, network, filesystem,
  process, environment, and clock access are absent unless a future capability ADR
  introduces a mediated operation.
- Each plugin instance runs in its own Web Worker. The host validates imports,
  manifest, protocol messages, sizes, and declared capabilities. It enforces time
  limits by terminating the worker and memory limits through the imported memory's
  maximum. Guest output is inert data, never code or HTML.
- The guest receives versioned JSON events and returns capability-checked effects.
  The trusted host resolves queries and applies semantic edits. Plugins never receive
  the frontend store, backend DTOs, physical graph paths, persistence primitives, or
  raw Tauri commands. Every graph write uses Tine's existing audited persistence and
  conflict/undo path.
- Manifests explicitly list supported platforms. Omission means desktop only;
  individual contributions may narrow that further. The first release may expose
  only desktop installation UI, but the runtime and API may not depend on desktop
  processes or JIT facilities so Android can follow without a second plugin model.
- Community plugins and Tine Labs describe different commitments. “Community” is
  distribution and trust metadata even when Martin publishes the plugin; Labs is
  reserved for experiments plausibly headed for core. Core may later supersede a
  plugin, and users must be able to disable either plugin without changing graph
  readability.
- The public registry records immutable package digests, source and license links,
  platform/capability declarations, deterministic checks, and an automated audit
  report for every version. Low-risk passing submissions may publish automatically;
  uncertain or elevated submissions quarantine automatically. Signed revocations
  can disable a known-malicious version. Source availability and a recognized
  open-source license are required; AI authorship is welcomed and disclosed, not
  treated as a security signal.
- Full `@logseq/libs` or Obsidian API compatibility is not a goal. Porting guides and
  agent-ready conformance tools shift conversion work to plugin authors.

## Consequences

Ordinary plugins cannot make arbitrary interfaces, call services, or run native
tools, but a compromised or poorly generated plugin has a much smaller blast radius
and the same runtime can work on desktop and mobile. Features must be expressed as
small host-owned contribution points and semantic operations, which also lets Tine
refactor internals without exposing them as public API.

The ABI, capability vocabulary, registry policy, and audit format remain explicitly
experimental through plugin API 0.x. Breaking changes may disable an incompatible
plugin with a clear explanation; persisted plugin-owned data must remain inert and
readable. Privileged network, filesystem, process, custom-DOM, and synchronization
plugins are deferred to later ADRs rather than smuggled into version 1.
