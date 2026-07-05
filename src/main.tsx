import { render } from "solid-js/web";
import { App } from "./App";
import { restoreSession } from "./router";
import { initParser } from "./render/parse";
import { applyTheme, applyAccent } from "./ui";
import "@fontsource/inter/400.css";
import "@fontsource/inter/500.css";
import "@fontsource/inter/600.css";
import "@fontsource/inter/700.css";
// Emoji are rendered as Twemoji SVG <img>s (see render/emoji.tsx), NOT via a
// color-emoji *font* — WebKitGTK paints a color webfont as a blank glyph.
import "katex/dist/katex.min.css";
import "pdfjs-dist/web/pdf_viewer.css";
import "./styles/theme.css";
import "./lsShimInstall";
import "./styles/app.css";

applyTheme();
applyAccent();

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
