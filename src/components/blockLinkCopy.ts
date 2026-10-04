// Edit-mode "copy a link to this block" commands: builtin Mod+C with no text
// selected copies `((uuid))` (OG parity), and Mod+Shift+C copies the embed
// `{{embed ((uuid))}}` (GH #279). Both persist the block's id:: through the
// ordinary guarded save before the link is handed out, so a pasted link never
// points at an id that exists only in memory.
import { writeClipboardText } from "../clipboard";
import { ensureBlockId } from "../document";
import { captureBinding, stillBound } from "../binding";
import { pushToast } from "../toasts";
import { backend } from "../backend";
import { ownedWhen, readOwned, writeOwned } from "../owned";
import { blockLink, graphLink, pageLink } from "../deepLinks";

export type BlockLinkKind = "ref" | "embed";

const TEXT: Record<BlockLinkKind, { wrap: (uuid: string) => string; ok: string; noun: string }> = {
  ref: { wrap: (uuid) => `((${uuid}))`, ok: "Copied block ref", noun: "reference" },
  embed: { wrap: (uuid) => `{{embed ((${uuid}))}}`, ok: "Copied block embed", noun: "embed" },
};

/** Copy references/embeds for one block or an ordered selection. Every ID is
 * saved before one clipboard write; a save/clipboard failure toasts without
 * publishing a partial selection. Failures toast and resolve; previous ID saves
 * remain. Empty arrays do nothing; repeats retain their input order. Multiple
 * refs are Markdown bullets, embeds newline-separated; single output is bare.
 * No undo step is added. Graph switches prevent subsequent saves/publication,
 * but cannot cancel a native clipboard write already in flight.
 * O(selected block bytes + their guarded page saves + clipboard bytes). */
export async function copyBlockLink(target: string | readonly string[], kind: BlockLinkKind): Promise<void> {
  const ids = typeof target === "string" ? [target] : [...target];
  if (!ids.length) return;
  const binding = captureBinding();
  const text = TEXT[kind];
  const refs: string[] = [];
  try {
    for (const id of ids) {
      if (!stillBound(binding)) return;
      const uuid = await ensureBlockId(id);
      if (!stillBound(binding)) return;
      if (!uuid) {
        pushToast(`Couldn't save the block id — ${text.noun} not copied.`, "error");
        return;
      }
      refs.push((ids.length > 1 && kind === "ref" ? "- " : "") + text.wrap(uuid));
    }
    await writeClipboardText(refs.join("\n"));
    if (stillBound(binding)) pushToast(ids.length > 1 ? `${text.ok}s` : text.ok, "success");
  } catch {
    if (stillBound(binding)) pushToast(`Couldn't copy block ${kind}: save or clipboard write failed.`, "error");
  }
}

// GH #181: external Tine links are references too, so they stamp through the same door.
/** Copy external links. O(selection saves + graph identity + clipboard bytes).
 * Blocks assign id:: via the existing document door; publication waits for all
 * saves. Graph/page copy never changes page content. Failures toast and resolve. */
export async function copyTineLink(target: { page: string } | { blocks: readonly string[] } | { blockUuid: string } | { root?: string }): Promise<void> {
  const binding = captureBinding();
  const owner = ownedWhen(() => stillBound(binding));
  try {
    const api = backend();
    if (!api.tineLinks?.identity) throw new Error("Copy link is available in the Tine app");
    const identity = await writeOwned(owner, api.tineLinks.identity("root" in target ? target.root : undefined));
    if (identity.kind === "stale") return;
    const id = identity.value;
    let text: string;
    if ("page" in target) text = pageLink(target.page, id);
    else if ("blocks" in target) {
      const links: string[] = [];
      for (const block of target.blocks) {
        const uuid = await ensureBlockId(block);
        if (!stillBound(binding)) return;
        if (!uuid) throw new Error("Couldn't save the block id");
        links.push(blockLink(uuid));
      }
      if (!links.length) return;
      text = links.join("\n");
    } else if ("blockUuid" in target) {
      const found = await readOwned(owner, api.resolveBlocks([target.blockUuid]));
      if (found.kind === "stale") return;
      if (!found.value[0]) throw new Error("Block no longer exists");
      text = blockLink(target.blockUuid);
    } else text = graphLink(id);
    await writeOwned(owner, writeClipboardText(text));
    if (stillBound(binding)) pushToast("Copied Tine link", "success");
  } catch (error) {
    if (stillBound(binding)) pushToast(`Couldn't copy Tine link: ${String(error)}`, "error");
  }
}
