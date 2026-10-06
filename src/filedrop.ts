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
import { formatForBlock, insertOutlineAfter, pageByName, trackAssetWrite, visibleOrder, withUndoUnit, node as docNode } from "./document";
import { pushToast } from "./toasts";
import { reportStaleAsset } from "./assetLanding";
import { captureBinding } from "./binding";
import { bindingOwner, readOwned, writeOwned } from "./owned";
import { graphMeta } from "./graphSession";
import type { OutlineNode } from "./editor/outline";
import { MAIN_WINDOW_ID, mainWindow, windowById } from "./windowRealm";

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

/** Install Tauri file-drop handling and return cleanup; outside Tauri return
 * an inert cleanup. A drop onto a block imports ordinary files as assets or
 * parses CSV/TSV into a grid (up to 5000 cells), then inserts blocks. Work
 * grows with dropped file bytes and visible blocks. Errors toast; imported
 * assets from earlier files can remain if a later step fails or the target
 * retires. Installation errors reject. */
export async function installFileDrop(): Promise<() => void> {
  let webview: ReturnType<typeof getCurrentWebview>;
  try {
    webview = getCurrentWebview();
  } catch {
    return () => {};
  }
  // `getCurrentWebview()` is MAIN's webview: these OS drop events and their
  // physical coordinates belong to the main window only. A workspace window's
  // drops are not wired (RECEIPT-OG-MW: product gap).
  const main = windowById(MAIN_WINDOW_ID) ?? mainWindow;
  const setActive = (on: boolean) => main.document.body.classList.toggle("file-drop-active", on);

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
    const dpr = main.devicePixelRatio || 1;
    const el = main.document.elementFromPoint(p.position.x / dpr, p.position.y / dpr);
    const onBlock = el?.closest("[data-block-id]")?.getAttribute("data-block-id") ?? null;
    const order = visibleOrder();
    const afterId = onBlock ?? order[order.length - 1] ?? null;
    if (!afterId || !docNode(afterId)) {
      pushToast("Drop a file onto a block to insert it.", "error");
      return;
    }
    const binding = captureBinding();
    const dropRoot = graphMeta()?.root;
    const dropPage = docNode(afterId).page;
    const owner = bindingOwner(() => graphMeta()?.root === dropRoot && docNode(afterId)?.page === dropPage);
    const pagePath = pageByName(dropPage)?.id;
    const format = formatForBlock(afterId);

    try {
      const nodes: OutlineNode[] = [];
      let storedAssets = 0;
      for (const path of paths) {
        const kind = delimitedKind(path);
        if (kind) {
          const result = await readOwned(owner, backend().readTextFile(path));
          if (result.kind === "stale") return;
          const matrix = parseDelimitedText(result.value, kind);
          const cells = delimitedCellCount(matrix);
          if (cells > MAX_DROPPED_CELLS) {
            pushToast(`"${basename(path)}" has ${cells} cells; CSV/TSV drops are limited to ${MAX_DROPPED_CELLS}.`, "error");
            continue;
          }
          nodes.push(matrixGridNode(titleWithoutExtension(path, kind), matrix));
          continue;
        }
        const orig = basename(path) || undefined;
        const result = await writeOwned(owner, trackAssetWrite(backend().importAsset(path, assetFileName(orig), binding.backendGeneration)));
        if (result.kind === "stale") { storedAssets++; continue; }
        const saved = result.value;
        storedAssets++;
        nodes.push({
          raw: assetMarkdown(saved, {
            label: orig,
            pagePath,
            format,
          }),
          children: [],
        });
      }
      if (!nodes.length) { if (storedAssets && !owner()) reportStaleAsset(); return; }
      if (!owner()) {
        if (storedAssets) reportStaleAsset();
        return;
      }
      const inserted = withUndoUnit("file-drop", [dropPage], () => insertOutlineAfter(afterId, nodes));
      if (!inserted) { pushToast("Dropped files could not be inserted at this outline depth.", "error"); return; }
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
