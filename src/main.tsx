import { render } from "solid-js/web";
import { App } from "./App";
import "./session";
import { restoreSession } from "./router";
import { initParser } from "./render/parse";
import { applyTheme, applyAccent } from "./ui";
import { initThemeGallery } from "./themeGallery";
import { initThemePackages } from "./themes/manager";
import "@fontsource/inter/400.css";
import "@fontsource/inter/500.css";
import "@fontsource/inter/600.css";
import "@fontsource/inter/700.css";
// Display emoji are Twemoji SVG <img>s (see render/emoji.tsx). Native editable
// controls cannot contain images, so they use this monochrome font instead of a
// system COLRv1 font, whose WebKitGTK/Skia path can abort the render process (#76).
import "@fontsource-variable/noto-emoji/wght.css";
import "katex/dist/katex.min.css";
import "pdfjs-dist/web/pdf_viewer.css";
import "./styles/theme.css";
import "./lsShimInstall";
import "./styles/app.css";

applyTheme();
applyAccent();
void initThemePackages().then(initThemeGallery);

// Restore the saved tab session before first paint, so tabs come back without a
// flash. Capped so a slow/stuck backend read can never block startup — worst
// case we paint the default journals tab and the session is simply not restored.
const mount = () => render(() => <App />, document.getElementById("root")!);
// Init the in-browser wasm parser before first paint so blocks render
// synchronously (no IPC, no fallback flash). Runs concurrently with the (capped)
// session restore; a parser-init failure is caught so it can't block startup —
// the legacy fallback renderer still covers that case during the transition.
void Promise.all([
  initParser().catch((e) => console.error("lsdoc-wasm init failed:", e)),
  Promise.race([restoreSession(), new Promise((r) => setTimeout(r, 1500))]),
]).then(mount, mount);
