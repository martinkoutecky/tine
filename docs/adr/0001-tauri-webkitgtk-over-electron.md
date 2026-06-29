# 0001. Tauri + the OS webview (WebKitGTK) instead of Electron

- **Status:** Accepted
- **Date:** 2026-06-29

## Context

Tine exists because Logseq gets sluggish on large graphs. Logseq is Electron +
DataScript: every instance ships a full Chromium and a Node runtime, and the UI
re-renders heavily. The single biggest lever on "feels fast and light" is the app
shell — how much runtime is between the user and the OS, and how much memory the
window costs at rest. The realistic choices were another Electron app (familiar,
heavy), or a native-webview shell (Tauri/Wails — small, but you inherit the host
webview's quirks instead of a bundled, uniform Chromium).

## Decision

We will build Tine on **Tauri 2**, rendering in the **OS-provided webview**
(WebKitGTK on Linux, WKWebView on macOS, WebView2 on Windows), with a Rust backend —
not Electron, and not a bundled browser engine.

## Consequences

- **Easier:** tiny binary and low idle memory vs Electron; a real Rust backend for
  the core (see [0003](0003-pure-rust-core-thin-ipc.md)); the perf story Tine is
  sold on.
- **Harder:** we own the host-webview quirks, and **WebKitGTK is the sharp edge** —
  blob-URL video can't play, color-emoji webfonts paint blank (we ship Twemoji
  SVGs instead), `localStorage` isn't persisted across launches, `window.confirm`
  is a silent no-op, and GPU/DMABUF compositor combos can fail to start (hence
  `TINE_GPU=0`). Each is worked around in code; new UI must be checked against
  WebKitGTK, not just a dev browser.
- **Committed to:** Linux is the primary, best-tested target; macOS/Windows ride
  the same frontend but are newer. Unsigned-build friction (Gatekeeper, SmartScreen)
  is now ours to document and, eventually, to fix with notarization.
