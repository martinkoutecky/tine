import { render } from "solid-js/web";
import { App } from "./App";
import { restoreSession } from "./router";
import { applyTheme, applyAccent } from "./ui";
import "@fontsource/inter/400.css";
import "@fontsource/inter/500.css";
import "@fontsource/inter/600.css";
import "@fontsource/inter/700.css";
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
