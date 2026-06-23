// Quick-capture mini window. A separate Tauri webview (capture.html) that the
// running app pops on a `tine --capture` signal. It hosts the SAME block Editor
// the main app uses — so `[[`/`#`/`((`/`/` autocomplete, slash commands and
// formatting behave identically — backed by an isolated single-block scratch
// store. On submit it emits the block as outline markdown; the MAIN window turns
// that into an append to today's journal (App.tsx), preserving the single-writer
// design (this window never writes the graph itself).
import { render } from "solid-js/web";
import { Show, createSignal, onMount } from "solid-js";
import { Editor } from "./components/Block";
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
import "./styles/app.css";
import "./styles/capture.css";

const SCRATCH = "·capture·";

function Capture() {
  const [rootId, setRootId] = createSignal("");

  // Seed an isolated single-block scratch page. We deliberately NEVER set
  // doc.loaded, so the persistence engine's scheduleSave/flush are no-ops and the
  // scratch is never written to disk. Autocomplete still works: it reads the
  // graph from the Rust backend (invoke), not from this store.
  const seed = () => {
    ensurePageLoaded({
      name: SCRATCH,
      kind: "page",
      title: SCRATCH,
      pre_block: null,
      blocks: [{ id: "", raw: "", collapsed: false, children: [] }],
      rev: null,
    });
    const rid = pageByName(SCRATCH)?.roots[0] ?? "";
    setEditingId(rid);
    setRootId(rid);
  };

  // Reset the scratch IN PLACE — keep the SAME block id so the mounted Editor
  // never references a deleted node: drop any extra blocks a template/paste made,
  // blank the first block, and put editing focus back on it.
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

  const hideWindow = async () => {
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      await getCurrentWindow().hide();
    } catch {
      // not in Tauri (dev preview)
    }
  };

  const submit = () => {
    const page = pageByName(SCRATCH);
    const md = page ? page.roots.map((r) => blockSubtreeMarkdown(r)).join("\n").trim() : "";
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

  onMount(() => {
    // Populate the keybinding table so editor shortcuts (bold/italic/…) work the
    // same as the main window. Its global handler is harmless here — the
    // switcher/panels it references don't exist in this window.
    installKeybindings();
    seed();
    void (async () => {
      try {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        // Dismiss on blur (click-away / alt-tab). The draft is preserved — only
        // Esc or submit clear it — so an accidental focus loss can't lose text.
        await getCurrentWindow().onFocusChanged(({ payload: focused }) => {
          if (!focused) void hideWindow();
        });
      } catch {
        // not in Tauri
      }
    })();
  });

  return (
    <div class="capture-shell">
      <Show when={rootId()} keyed>
        {(rid) => <Editor id={rid} onSubmit={submit} onCancel={cancel} />}
      </Show>
    </div>
  );
}

render(() => <Capture />, document.getElementById("capture-root")!);
