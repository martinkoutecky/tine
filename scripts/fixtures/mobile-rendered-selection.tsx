import { render } from "solid-js/web";
import { For, Show } from "solid-js";
import { initParser } from "../../src/render/parse";
import { loadSingle } from "../../src/document/workingSet";
import { node, pageByName } from "../../src/document/model";
import { editingId, startEditing } from "../../src/editorController";
import { Block } from "../../src/components/Block";
import { MobileKeyboardToolbar } from "../../src/components/MobileKeyboardToolbar";
import { RenderedSelectionToolbar } from "../../src/components/RenderedSelectionToolbar";
import "../../src/styles/inter.css";
import "../../src/styles/theme.css";
import "../../src/styles/app.css";
import "../../src/styles/topbar.css";
import "../../src/styles/touchGestures.css";
import "../../src/styles/ls-shim.css";
import "../../src/styles/help.css";

// A mobile-shaped page for scripts/shot-gh375-rendered-selection.mjs: the real
// Block renderer, the real keyboard dock and rendered-selection toolbar, the help
// corner's "?" (which must clear the docked toolbar), the
// shipped CSS, a scroller shaped like the app's `.main-content`, and (with
// `?nav`) a bottom navigation row like Settings > Navigation buttons at the
// bottom. The harness plays the IME by resizing the viewport and dispatching
// `tine-ime`, as MainActivity does on Android.
document.documentElement.dataset.theme = "light";
await initParser();
const names = ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india", "juliet",
  "kilo", "lima", "mike", "november", "oscar", "papa", "quebec", "romeo", "sierra", "tango", "uniform",
  "victor", "whiskey", "xray", "yankee", "zulu"];
loadSingle({ name: "Long press", title: "Long press", kind: "page", pre_block: null, format: "md",
  blocks: names.map((n, i) => ({ id: `b${i}`, raw: `${n} selectable words in block ${i}`, collapsed: false, children: [] })) });
(window as unknown as { __tine: unknown }).__tine = { startEditing, editingId, raw: (id: string) => node(id)?.raw };
const nav = new URLSearchParams(location.search).has("nav");
render(() => (
  <div style={{ display: "flex", "flex-direction": "column", height: "100dvh", width: "100vw" }}>
    <main class="main-content" style={{ padding: "0 12px", flex: "1 1 auto", overflow: "auto" }}>
      <For each={pageByName("Long press")!.roots}>{id => <Block id={id} />}</For>
    </main>
    <Show when={nav}><nav class="mobile-navigation-toolbar">nav</nav></Show>
    <MobileKeyboardToolbar />
    <RenderedSelectionToolbar />
    <div class="help-corner"><button class="help-corner-btn" type="button">?</button></div>
  </div>
), document.getElementById("root")!);
