import { render } from "solid-js/web";
import { App } from "./App";
import { restoreSession } from "./router";
import { applyTheme, applyAccent } from "./ui";
import "@fontsource/inter/400.css";
import "@fontsource/inter/500.css";
import "@fontsource/inter/600.css";
import "@fontsource/inter/700.css";
// Bundle a COLOR emoji font so `icon::` emoji (and any emoji in notes) render in
// WebKitGTK, which otherwise shows ▯ tofu boxes when no system emoji font exists.
// Subsetted by unicode-range, so the browser only loads the slice it actually
// needs — but it does add the font files (~10MB on disk) to the bundle.
import "@fontsource/noto-color-emoji";
import "katex/dist/katex.min.css";
import "pdfjs-dist/web/pdf_viewer.css";
import "./styles/theme.css";
import "./styles/app.css";

applyTheme();
applyAccent();

// Restore the saved tab session before first paint, so tabs come back without a
// flash. Capped so a slow/stuck backend read can never block startup — worst
// case we paint the default journals tab and the session is simply not restored.
const mount = () => render(() => <App />, document.getElementById("root")!);
void Promise.race([
  restoreSession(),
  new Promise((r) => setTimeout(r, 1500)),
]).then(mount, mount);
