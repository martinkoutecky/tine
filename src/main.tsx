import { render } from "solid-js/web";
import { App } from "./App";
import "./session";
import { restoreSession } from "./router";
import { initParser } from "./render/parse";
import { applyTheme, applyAccent, pushToast } from "./ui";
import { startCommunityExtensions } from "./plugins/startup";
import { isTauri } from "./backend";
import { getCurrentWindow } from "@tauri-apps/api/window";
// Full upstream Inter variable fonts retain OpenType stylistic sets/character
// variants. Fontsource's per-script static subsets stripped them (GH #298).
import "./styles/inter.css";
// Linux display emoji use Twemoji SVGs; editable controls use this safe fallback
// instead of system COLRv1 (WebKitGTK/Skia can abort, #76). Windows/Apple share
// their native color face between display and editing, with this fallback.
import "@fontsource-variable/noto-emoji/wght.css";
import "katex/dist/katex.min.css";
import "pdfjs-dist/web/pdf_viewer.css";
import "./styles/theme.css";
import "./lsShimInstall";
import "./styles/app.css";
import { installEditableEmojiPlatform } from "./editableEmoji";
import { applyContentWidths } from "./contentWidth";
import { installSystemInsetOwner } from "./systemInsets";
import { installPlatformAttribute } from "./nativeChrome";

installPlatformAttribute();
installSystemInsetOwner();
installEditableEmojiPlatform();
applyTheme();
applyAccent();
applyContentWidths();
const communityExtensionsReady = startCommunityExtensions()
  .then(({ pluginInitialization }) => {
    void pluginInitialization.catch((error) =>
      pushToast(`Plugins unavailable: ${String(error)}`, "error")
    );
  })
  .catch((error) => pushToast(`Community extensions unavailable: ${String(error)}`, "error"));

// Restore the saved tab session before first paint, so tabs come back without a
// flash. Capped so a slow/stuck backend read can never block startup — worst
// case we paint the default journals tab and the session is simply not restored.
async function revealMainWindowAfterStableFrame(): Promise<void> {
  if (!isTauri()) return;
  // The window starts hidden, so the user never sees the default white webview,
  // unthemed controls, or an empty root. A hidden WebKit view may throttle
  // requestAnimationFrame indefinitely, so wait one microtask after Solid mounts
  // the themed App DOM, then map the native window; its first compositor frame
  // is the complete application rather than the backing surface.
  await new Promise<void>((resolve) => queueMicrotask(resolve));
  await getCurrentWindow().show();
}

const mount = () => {
  const root = document.getElementById("root")!;
  // index.html owns the immediate, dependency-free readiness frame. Remove it
  // only when Solid is ready to synchronously install the real application.
  root.replaceChildren();
  render(() => <App />, root);
  void revealMainWindowAfterStableFrame().catch((error) =>
    console.error("failed to reveal the main window:", error)
  );
};
// Init the in-browser wasm parser before first paint so blocks render
// synchronously (no IPC, no fallback flash). Runs concurrently with the (capped)
// session restore; a parser-init failure is caught so it can't block startup —
// the legacy fallback renderer still covers that case during the transition.
void Promise.all([
  initParser().catch((e) => console.error("lsdoc-wasm init failed:", e)),
  Promise.race([restoreSession(), new Promise((r) => setTimeout(r, 1500))]),
  communityExtensionsReady,
]).then(mount, mount);
