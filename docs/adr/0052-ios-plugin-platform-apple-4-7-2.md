# 0052. The plugin API may never re-export native surface (Apple guideline 4.7.2)

- **Status:** Accepted
- **Date:** 2026-08-08
- **Applies to:** any change that widens `PLUGIN_CAPABILITIES`, the guest host
  protocol, or the plugin platform list. Read this **before** adding a capability.

## Context

0045 established plugins as capability-limited WebAssembly guests; 0047 added
Android through explicit opt-in and deliberately left `ios` as an API vocabulary
value rather than a shipped host. Scoping an actual iOS app (2026-08-08) surfaced
an Apple constraint that constrains the plugin architecture itself, not merely
whether an iOS host exists.

Apple **App Review Guideline 4.7** explicitly permits "HTML5 and JavaScript mini
apps and mini games, streaming games, chatbots, and **plug-ins**" that are not
embedded in the binary. That is the carve-out from 2.5.2's prohibition on
downloading executable code, and it is why plugins on iOS are possible at all —
Obsidian's iOS app ships an in-app community-plugin browser as a working existence
proof, and WKWebView JavaScript is JIT-enabled in Apple's separate WebContent
process, so there is no performance obstacle either.

But 4.7 comes with conditions, and **4.7.2** is the one with architectural teeth:

> plug-ins ... **may not extend or expose native platform APIs or technologies to
> the software without prior permission from Apple**

A plugin API that hands guests the Tauri command surface, filesystem handles, or
any other native capability violates this. Retrofitting that split after an API
has grown is expensive and breaks published plugins, which is precisely why this
is recorded now, while iOS plugins are still deferred.

**Tine is currently compliant by construction, and that is not an accident to
rely on silently.** `PLUGIN_CAPABILITIES` (`src/plugins/manifest.ts`) is a closed
vocabulary of host-mediated verbs — `commands.register`, `graph.read.visible`,
`graph.write.block`, `settings.read`, and so on. None re-exports a native API;
manifest parsing rejects anything outside the list (`manifest.test.ts` asserts an
unknown capability throws). The risk is not today's design. The risk is a future
capability added for a good local reason that quietly punches through the
boundary.

## Decision

- **The plugin capability vocabulary is host-mediated only.** Every capability
  must name a semantic operation Tine performs *on the guest's behalf*. No
  capability may hand a guest native platform surface — no Tauri `invoke`, no raw
  filesystem or path handles, no process, network-socket, shell, or OS-service
  access, and no pass-through that lets a guest name a native command.
- **This holds on every platform, not just iOS.** A desktop-only escape hatch
  would still have to be removed before an iOS host could ship, and would strand
  any plugin built on it. There is no "desktop-only native capability" tier.
- **`ios` stays an API vocabulary value and is not a shipped host.** iOS v1 ships
  with plugins off (see the iOS scoping note). Turning the iOS plugin host on is a
  separate decision that must revisit this ADR, plus 4.7.4 (a published index with
  universal links to every plugin), 4.7.1/4.7.5 (filtering, report/block, age
  rating), and 3.2.2(i) (present the catalogue inside Settings, never as a
  "store").
- **A registry is not a storefront.** The current Apple Developer Program License
  Agreement §3.3.1(B) forbids creating "a store or storefront for other
  **Applications**" — narrowed from the older "other code or applications". A
  Tine-plugin catalogue is not what that prohibits. Do not treat the registry
  itself as the blocker; 4.7.2 is the blocker.
- **If a genuinely native capability is ever required**, the options are: extend
  the *host* with a new semantic verb the guest requests (preferred, keeps the
  boundary), or accept that the plugin is desktop-only *and* that iOS plugin
  support is thereby forfeited — a product decision for Martin, not an
  implementation detail.

## Consequences

Plugin authors get one portable API and no conditional guest code, and the door to
iOS plugins stays open at no ongoing cost — the constraint costs nothing while it
is respected and is very expensive to recover once broken.

The cost is that some plugin ideas needing native reach cannot be served by
widening the capability list; they need a host-side verb instead, which is more
work per feature. That is the intended trade.

`PLUGIN_CAPABILITIES` carries a pointer to this ADR, and the manifest tests pin
the closed vocabulary, so a change that widens the boundary has to pass this
decision rather than discover it afterwards.

## References

- ADR 0045 (capability-limited Wasm guests), 0046 (declarative settings/themes),
  0047 (Android opt-in; origin of `ios` as vocabulary-only).
- Apple App Review Guidelines 4.7, 4.7.1, 4.7.2, 4.7.4, 4.7.5, 2.5.2, 3.2.2(i)
  (guidelines as of their "Last Updated: June 8, 2026" revision).
- Apple Developer Program License Agreement §3.3.1(B).
- Private scoping note and full checklist:
  `tine-agents/specs/notes/2026-08-08-ios-scoping.md`.
- Research receipt:
  `tine-master/subagent-tasks/notes/2026-08-08-ios-appstore-policy.md`.
