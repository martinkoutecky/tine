// Quick-capture mini window. A separate Tauri webview (capture.html) that the
// running app pops on a `tine --capture` signal. It renders the SAME block tree
// the main app uses (Block + Editor) over an isolated scratch store — so all
// autocomplete (`[[`/`#`/`((`/`/`), slash commands, the date picker and
// formatting behave identically, and Enter can create real new blocks. On submit
// the scratch is serialized to outline markdown and emitted; the MAIN window
// appends it to today's journal (App.tsx), keeping the single-writer design (this
// window never writes the graph itself — it never sets doc.loaded, so the
// persistence engine's saves are no-ops here).
import { render } from "solid-js/web";
import { For, Show, createSignal, onMount } from "solid-js";
import { Block, CaptureCtx, type CaptureApi } from "./components/Block";
import { DatePicker } from "./components/DatePicker";
import {
  ensurePageLoaded,
  pageByName,
  blockSubtreeMarkdown,
  setEditingId,
  deleteBlock,
  setRaw,
  doc,
} from "./store";
import { installKeybindings } from "./keybindings";
import { backend } from "./backend";
import "./styles/app.css";
import "./styles/capture.css";

const SCRATCH = "·capture·";

function Capture() {
  const [ready, setReady] = createSignal(false);
  const [enterFiles, setEnterFiles] = createSignal(false);

  const roots = () => pageByName(SCRATCH)?.roots ?? [];

  const seed = () => {
    ensurePageLoaded({
      name: SCRATCH,
      kind: "page",
      title: SCRATCH,
      pre_block: null,
      blocks: [{ id: "", raw: "", collapsed: false, children: [] }],
      rev: null,
    });
    setEditingId(pageByName(SCRATCH)?.roots[0] ?? null);
    setReady(true);
  };

  // Reset to a single empty block (the page stays loaded; we just clear it).
  const clearScratch = () => {
    const page = pageByName(SCRATCH);
    const rid = page?.roots[0];
    if (!page || !rid) return;
    for (const r of page.roots.slice(1)) deleteBlock(r);
    const first = doc.byId[rid];
    if (first) for (const c of [...first.children]) deleteBlock(c);
    setRaw(rid, "");
    setEditingId(rid);
  };

  // The window is created hidden at startup, so the editor's onMount fit to a
  // 0-height layout and focus was a no-op. Re-fit + focus the live textarea when
  // the window is actually shown.
  const refit = () => {
    const ta = document.querySelector<HTMLTextAreaElement>(".capture-shell textarea");
    if (!ta) return;
    ta.focus();
    ta.style.height = "auto";
    ta.style.height = `${ta.scrollHeight}px`;
  };

  const hideWindow = async () => {
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      await getCurrentWindow().hide();
    } catch {
      // not in Tauri (dev preview)
    }
  };

  const loadPref = () => {
    void backend().getCaptureEnterFiles().then(setEnterFiles).catch(() => {});
  };

  const submit = () => {
    const md = roots().map((r) => blockSubtreeMarkdown(r)).join("\n").trim();
    void (async () => {
      if (md) {
        try {
          const { emit } = await import("@tauri-apps/api/event");
          await emit("quick-capture", { text: md });
        } catch {
          // ignore — emit only works inside Tauri
        }
      }
      clearScratch();
      await hideWindow();
    })();
  };

  const cancel = () => {
    clearScratch();
    void hideWindow();
  };

  const captureApi: CaptureApi = { submit, cancel, enterFiles };

  onMount(() => {
    // Populate the keybinding table so editor shortcuts (bold/italic/…) work the
    // same as the main window. Its global handler is harmless here.
    installKeybindings();
    loadPref();
    seed();
    void (async () => {
      try {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        await getCurrentWindow().onFocusChanged(({ payload: focused }) => {
          if (focused) {
            loadPref(); // pick up a Settings change made while we were hidden
            requestAnimationFrame(refit);
          } else {
            // Dismiss on blur. The draft is preserved (only Esc/submit clear it)
            // so an accidental focus loss can't lose text.
            void hideWindow();
          }
        });
      } catch {
        // not in Tauri
      }
    })();
  });

  return (
    <CaptureCtx.Provider value={captureApi}>
      <div class="capture-shell">
        <Show when={ready()}>
          <For each={roots()}>{(rid) => <Block id={rid} />}</For>
        </Show>
      </div>
      <DatePicker />
    </CaptureCtx.Provider>
  );
}

render(() => <Capture />, document.getElementById("capture-root")!);
