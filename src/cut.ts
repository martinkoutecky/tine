import { captureBinding, bindingCurrent } from "./binding";
import { cancelClipboardCutGrant, clipboardWriteRevision, copyBlockOutline, peekClipboardPayload } from "./clipboard";
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

/** Write plain text and outline HTML, then remove only if the graph, currentText,
 * selected subtrees, source-page generations and clipboard write token still match.
 * The private payload may be absent for empty/invalid or oversized selections;
 * ownership still applies. A mismatch silently leaves a copy. A failed write
 * rejects before removal; callback failure rejects, possibly after the copy.
 * O(selected descendants + raw/text bytes), plus native clipboard latency. */
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
  const ownership = clipboardWriteRevision();
  const generation = peekClipboardPayload()?.generation;
  await write;
  const sameSource = bindingCurrent(binding)
    && currentText() === text
    && sourceSnapshot(ids) === snapshot
    && JSON.stringify(buildClipboardPayload(ids)?.sourcePages) === sourcePages;
  const stillOwned = clipboardWriteRevision() === ownership
    && (generation === undefined || peekClipboardPayload()?.generation === generation);
  if (sameSource && stillOwned) {
    remove();
    if (generation !== undefined && ids.some((id) => docNode(id))) cancelClipboardCutGrant(generation);
  } else if (generation !== undefined) cancelClipboardCutGrant(generation);
}
