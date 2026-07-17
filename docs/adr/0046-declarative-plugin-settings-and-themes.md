# 0046. Plugin settings and themes are separate declarative host contracts

- **Status:** Accepted
- **Date:** 2026-07-12

## Context

Tine's experimental plugin API 0.1 provided namespaced scalar storage but no way
for a plugin to declare user-facing settings. Rendering plugin-owned HTML would
weaken the Wasm boundary and make internal UI structure part of the extension API.
Themes have a different lifecycle and risk profile from executable plugins: treating
them as zero-capability plugins would still conflate one-selected presentation data
with independently enabled behavior.

AI-assisted ports also need durable provenance. A behavioral reimplementation may
share no source code with a Logseq or Obsidian extension, but users and auditors must
still be able to identify the original behavior, authors, license, and exact source
revision that informed it.

## Decision

- Plugin API 0.2 may declare a bounded settings schema. Tine renders every control,
  validates defaults and persisted values, namespaces storage by plugin id, and
  delivers a complete snapshot on activation and after user changes. Plugins cannot
  supply HTML, CSS, scripts, secret fields, arbitrary objects, or custom controls.
- Initial settings are device-local. Disabled plugins remain configurable. Resetting
  restores schema defaults; uninstalling the last version removes stored values and
  never writes plugin metadata into graph files.
- Marketplace themes are a separate theme API 0.1 package type. A theme is inert,
  one-selected presentation data under Appearance, not Wasm. Initial community
  themes may set only host-whitelisted semantic tokens and bounded theme settings;
  remote resources, arbitrary selectors, imports, and executable content are absent.
  User-owned `logseq/custom.css` remains the advanced escape hatch and loads last.
- The plugin and theme registries share immutable digests, signing, audit reports,
  revocation, screenshots, and source/license requirements, but retain distinct
  manifests, installation state, activation rules, and catalogue categories.
- A port may declare structured `portedFrom` provenance: ecosystem, original name,
  source URL, immutable revision, license, authors, and whether it is a behavioral
  reimplementation or a source-derived port. Registry automation verifies that this
  metadata is present before Tine markets an artifact as a port.
- Agent tooling must either produce a conforming package or a structured minimal API
  gap report. It must not widen authority or emulate legacy APIs to force a port.

## Consequences

Settings and themes remain usable on mobile and survive Tine refactors because the
host owns rendering. Theme expressiveness is intentionally narrower than arbitrary
CSS, and some popular themes can only be translated by preserving their semantic
design rather than their selectors. Plugin API 0.1 packages must be rebuilt for 0.2
before public launch; no compatibility promise has yet been made for the experimental
0.1 format.
