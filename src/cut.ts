import { captureBinding, stillBound } from "./binding";
import { cancelClipboardCutGrant, copyBlockOutline, peekClipboardPayload } from "./clipboard";
import { buildClipboardPayload, node as docNode } from "./document";

function sourceSnapshot(ids: string[]): string {
  const blocks: unknown[] = [];
  const pending = [...ids];
  const visited = new Set<string>();
  while (pending.length) {
    const id = pending.pop()!;
    if (visited.has(id)) continue;
    visited.add(id);
    const node = docNode(id);
    blocks.push(node
      ? [id, node.raw, node.page, node.parent, node.children]
      : [id, null]);
    if (node) for (const child of node.children) pending.push(child);
  }
  return JSON.stringify(blocks);
}

/** Copy first; remove only the same source while this Cut still owns the clipboard. */
export async function cutBlocks(
  ids: string[],
  text: string,
  currentText: () => string,
  remove: () => void,
): Promise<void> {
  const binding = captureBinding();
  const payload = buildClipboardPayload(ids);
  const snapshot = sourceSnapshot(ids);
  const sourcePages = JSON.stringify(payload?.sourcePages);
  const write = copyBlockOutline("cut", text, payload);
  const generation = peekClipboardPayload()?.generation;
  await write;
  const sameSource = stillBound(binding)
    && currentText() === text
    && sourceSnapshot(ids) === snapshot
    && JSON.stringify(buildClipboardPayload(ids)?.sourcePages) === sourcePages;
  const stillOwned = generation === undefined || peekClipboardPayload()?.generation === generation;
  if (sameSource && stillOwned) {
    remove();
    if (generation !== undefined && ids.some((id) => docNode(id))) cancelClipboardCutGrant(generation);
  } else if (generation !== undefined) cancelClipboardCutGrant(generation);
}
