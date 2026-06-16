import { render } from "solid-js/web";
import { App } from "./App";
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
render(() => <App />, document.getElementById("root")!);
