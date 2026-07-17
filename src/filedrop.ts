// OS file drag-and-drop → insert as graph assets. Tauri captures the native file
// drop (so HTML drag events never reach the webview) and hands us the absolute
// paths + a physical-pixel drop position; we map that to the block under the
// cursor and insert each file as a new sibling block (timestamped name + the
// `![](../assets/…)` media form, like the picker/paste paths).

import { getCurrentWebview } from "@tauri-apps/api/webview";
import { backend } from "./backend";
import { assetFileName, assetMarkdown } from "./media";
import { matrixGridNode, delimitedCellCount } from "./sheet/conversions";
import { parseDelimitedText, type DelimitedKind } from "./sheet/tsv";
import { doc, formatForBlock, insertOutlineAfter, pageByName, trackAssetWrite, visibleOrder, withUndoUnit } from "./store";
import { pushToast } from "./ui";
import type { OutlineNode } from "./editor/outline";

const MAX_DROPPED_CELLS = 5000;

function basename(path: string): string {
  return path.split(/[\\/]/).pop() ?? "";
}

function delimitedKind(path: string): DelimitedKind | null {
  const lower = basename(path).toLowerCase();
  if (lower.endsWith(".csv")) return "csv";
  if (lower.endsWith(".tsv")) return "tsv";
  return null;
}

function titleWithoutExtension(path: string, kind: DelimitedKind): string {
  const name = basename(path);
  return name.slice(0, Math.max(0, name.length - kind.length - 1)) || "Dropped table";
}

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
        const kind = delimitedKind(path);
        if (kind) {
          const text = await backend().readTextFile(path);
          const matrix = parseDelimitedText(text, kind);
          const cells = delimitedCellCount(matrix);
          if (cells > MAX_DROPPED_CELLS) {
            pushToast(`"${basename(path)}" has ${cells} cells; CSV/TSV drops are limited to ${MAX_DROPPED_CELLS}.`, "error");
            continue;
          }
          nodes.push(matrixGridNode(titleWithoutExtension(path, kind), matrix));
          continue;
        }
        const orig = basename(path) || undefined;
        const saved = await trackAssetWrite(backend().importAsset(path, assetFileName(orig)));
        const page = pageByName(doc.byId[afterId].page);
        nodes.push({
          raw: assetMarkdown(saved, {
            label: orig,
            pagePath: page?.path,
            format: formatForBlock(afterId),
          }),
          children: [],
        });
      }
      if (!nodes.length) return;
      withUndoUnit("file-drop", [doc.byId[afterId].page], () => insertOutlineAfter(afterId, nodes));
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
