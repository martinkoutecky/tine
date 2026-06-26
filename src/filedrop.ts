// OS file drag-and-drop → insert as graph assets. Tauri captures the native file
// drop (so HTML drag events never reach the webview) and hands us the absolute
// paths + a physical-pixel drop position; we map that to the block under the
// cursor and insert each file as a new sibling block (timestamped name + the
// `![](../assets/…)` media form, like the picker/paste paths).

import { getCurrentWebview } from "@tauri-apps/api/webview";
import { backend } from "./backend";
import { assetFileName, assetMarkdown } from "./media";
import { doc, insertOutlineAfter, visibleOrder } from "./store";
import { pushToast } from "./ui";
import type { OutlineNode } from "./editor/outline";

/** Install the OS file-drop handler. Returns an uninstaller. No-op outside the
 *  Tauri shell (browser mock / tests). */
export async function installFileDrop(): Promise<() => void> {
  let webview: ReturnType<typeof getCurrentWebview>;
  try {
    webview = getCurrentWebview();
  } catch {
    return () => {};
  }
  const setActive = (on: boolean) => document.body.classList.toggle("file-drop-active", on);

  const unlisten = await webview.onDragDropEvent(async (event) => {
    const p = event.payload;
    if (p.type === "enter" || p.type === "over") return setActive(true);
    if (p.type === "leave") return setActive(false);
    if (p.type !== "drop") return;
    setActive(false);
    const paths = p.paths ?? [];
    if (!paths.length) return;

    // Resolve the drop target: the block under the drop point, else the last
    // visible block (drops in the page whitespace land at the end). Tauri gives a
    // physical-pixel position; elementFromPoint wants CSS pixels.
    const dpr = window.devicePixelRatio || 1;
    const el = document.elementFromPoint(p.position.x / dpr, p.position.y / dpr);
    const onBlock = el?.closest("[data-block-id]")?.getAttribute("data-block-id") ?? null;
    const order = visibleOrder();
    const afterId = onBlock ?? order[order.length - 1] ?? null;
    if (!afterId || !doc.byId[afterId]) {
      pushToast("Drop a file onto a block to insert it.", "error");
      return;
    }

    try {
      const nodes: OutlineNode[] = [];
      for (const path of paths) {
        const orig = path.split(/[\\/]/).pop() || undefined;
        const saved = await backend().importAsset(path, assetFileName(orig));
        nodes.push({ raw: assetMarkdown(saved), children: [] });
      }
      insertOutlineAfter(afterId, nodes);
      pushToast(`Inserted ${nodes.length} file${nodes.length === 1 ? "" : "s"}`, "success");
    } catch (e) {
      pushToast(`Couldn't insert dropped file: ${String(e)}`, "error");
    }
  });

  return () => {
    setActive(false);
    unlisten();
  };
}
