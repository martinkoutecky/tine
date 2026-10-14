import { render } from "solid-js/web";
import { createSignal } from "solid-js";
import { initParser } from "../../src/render/parse";
import { loadSingle } from "../../src/document/workingSet";
import { Block } from "../../src/components/Block";
import { pluginManager } from "../../src/plugins/manager";
import type { PluginSettings } from "../../src/plugins/settings";
import { galleryThemes } from "../../src/styles/themes";
import "../../src/styles/inter.css";
import "../../src/styles/theme.css";
import "../../src/styles/app.css";
import "../../src/styles/ls-shim.css";

const [settings, setSettings] = createSignal<PluginSettings>({ display: "all", thickness: 1, color: "", "color-by-level": false });
pluginManager.hasDeclarativeDecoration = kind => kind === "thread-lines";
pluginManager.declarativeDecorationSetting = (_kind, key) => settings()[key];
Object.assign(window, { setThreadSettings: setSettings });
Object.assign(window, { setThreadTheme: (id: string) => {
  let style = document.getElementById("thread-theme");
  if (!style) { style = document.createElement("style"); style.id = "thread-theme"; document.head.append(style); }
  style.textContent = galleryThemes.find(theme => theme.id === id)?.css ?? "";
} });
await initParser();
loadSingle({ name: "Thread", title: "Thread", kind: "page", pre_block: null, format: "md", blocks: [
  { id: "root", raw: "Topmost ancestor", collapsed: false, children: [
    { id: "before", raw: "Unrelated preceding sibling", collapsed: false, children: [] },
    { id: "parent", raw: "Parent with wrapped text\nSecond line", collapsed: false, children: [
      { id: "middle", raw: "Middle ancestor", collapsed: false, children: [
        { id: "focus", raw: "Focused block", collapsed: false, children: [] },
        { id: "after", raw: "Unrelated following sibling", collapsed: false, children: [] },
      ] },
    ] },
    { id: "last", raw: "Another branch", collapsed: false, children: [] },
  ] },
] });
render(() => <main class="main-content" style={{ padding: "40px", width: "min(640px, calc(100vw - 80px))" }}><Block id="root" /></main>, document.getElementById("root")!);
