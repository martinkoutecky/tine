# 0047. Android joins the initial plugin-platform launch through explicit opt-in

- **Status:** Accepted
- **Date:** 2026-07-13

## Context

The Wasm guest boundary, declarative settings, themes, and host-owned effects do
not require desktop processes or JIT compilation. Tine's Android UI already exposes
the same Settings surfaces, while shipping desktop-only first would make “mobile
later” an untested architectural promise. A manifest platform list must still be an
honest compatibility claim: many author workflows will only have tested desktop.

## Decision

- Tine 0.6 exposes plugin and theme browse, install, enable/select, settings, disable,
  and uninstall flows on Android as well as desktop.
- Omitted plugin platforms continue to mean desktop only. A plugin must explicitly
  add `android`; individual contributions may narrow that declaration. The starter
  template stays desktop-only so an AI agent cannot claim mobile support by accident.
- Every launch plugin that declares Android must pass the same install, activation,
  behavior, settings/effect, disable, and uninstall host contract under an Android
  WebView user agent. A real signed APK and phone smoke test remain the final release
  evidence; a browser host simulation is not a substitute for hardware.
- `ios` remains an API vocabulary value for portable guests, not a promise that Tine
  0.6 ships an iOS host. No desktop-only authority may be added merely to preserve a
  plugin that cannot run on mobile.
- A plugin may support desktop while omitting Android entirely. Tine shows it as
  unavailable rather than trying to run it or silently hiding the incompatibility.

## Consequences

Community authors can intentionally opt out of mobile with no conditional guest
code, while portable plugins and inert themes work through one API and one package.
The release gate now includes Android-host plugin behavior and an installable APK.
Hardware-specific WebView, touch, lifecycle, and storage bugs can still exist, so
the 0.6 release waits for the documented phone smoke test before tagging.
