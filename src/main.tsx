import { render } from "solid-js/web";
import { App } from "./App";
import "./session";
import { installWorkspaceWindowSession } from "./session";
import { restoreSession } from "./router";
import { initParser } from "./render/parse";
import { applyTheme, applyAccent } from "./ui";
import { pushToast } from "./toasts";
import { startCommunityExtensions } from "./plugins/startup";
import { backend, isTauri } from "./backend";
import { installBackendClock } from "./journal";
import { isPublishedExport, loadPublishedSnapshot } from "./publishedBackend";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { installWorkspaceWindows, installWorkspaceWindowShell } from "./workspaceWindows";
import { renderWorkspaceWindowShell } from "./components/WorkspaceWindowShell";
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
import { installSystemInsetOwner } from "./systemInsets";
import { installPlatformAttribute } from "./nativeChrome";
import { installEditableEmojiPlatform } from "./editableEmoji";
import "./styles/editableEmoji.css";
import "./styles/app.css";
import "./styles/touchGestures.css"; // after app.css: its .sel-toolbar-mobile overrides .sel-toolbar
import "./styles/topbar.css";
import "./styles/readiness.css";
import "./styles/themePresentation.css";
import "./styles/pdf-workspace.css";
import "./styles/settingsControls.css";
import "./styles/query.css";
import "./styles/conflicts.css";
import "./styles/region-failure.css";
import "./styles/published.css";

// The ES5 check already explains GH #572; preserve its card and reveal the window.
if ((window as { __tineUnsupportedEngine?: boolean }).__tineUnsupportedEngine) {
  if (isTauri()) void getCurrentWindow().show().catch(() => console.error("failed to reveal unsupported-engine card"));
  throw new Error("Tine: unsupported web engine");
}

installPlatformAttribute();
if (isTauri()) installBackendClock(() => backend().localClock());
installSystemInsetOwner();
installEditableEmojiPlatform();
applyTheme();
applyAccent();
// Workspace windows (desktop): wired before the session restore reopens them.
installWorkspaceWindowShell(renderWorkspaceWindowShell);
if (!isPublishedExport()) installWorkspaceWindows(installWorkspaceWindowSession);
if (isPublishedExport()) document.documentElement.classList.add("tine-published");
const communityExtensionsReady = isPublishedExport() ? Promise.resolve() : startCommunityExtensions()
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
  // Start-minimized to the tray (GH #625): native created this window hidden on
  // purpose and shows it from the tray icon or a second launch. Nothing below
  // waits on a frame, so the app finishes loading while hidden.
  if ((globalThis as { __TINE_START_HIDDEN__?: boolean }).__TINE_START_HIDDEN__) return;
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
    console.error("failed to reveal the main window")
  );
};
const publishedSnapshotReady = isPublishedExport()
  ? loadPublishedSnapshot().then(() => undefined, (error) => {
      console.error("published snapshot unavailable");
      document.getElementById("root")!.textContent = "Couldn't load snapshot.json — serve this folder over HTTP";
      throw error;
    })
  : Promise.resolve();
// Init the in-browser wasm parser before first paint so blocks render
// synchronously (no IPC, no fallback flash). A parser-init failure is caught so
// it can't block startup. Session restore starts only after init settles:
// restoring parses saved query views through the synchronous WASM page-identity
// and group-field owners (OG-DUPF05), which must not run before init.
const parserSettled = initParser().catch(() => console.error("lsdoc-wasm init failed"));
void Promise.all([
  parserSettled,
  isPublishedExport() ? Promise.resolve() : parserSettled.then(() => Promise.race([restoreSession(), new Promise((r) => setTimeout(r, 1500))])),
  communityExtensionsReady,
  publishedSnapshotReady,
]).then(mount, () => { if (!isPublishedExport()) mount(); else console.error("published app startup failed"); });
