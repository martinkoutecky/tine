// Edit-mode "copy a link to this block" commands: builtin Mod+C with no text
// selected copies `((uuid))` (OG parity), and Mod+Shift+C copies the embed
// `{{embed ((uuid))}}` (GH #279). Both persist the block's id:: through the
// ordinary guarded save before the link is handed out, so a pasted link never
// points at an id that exists only in memory.
import { writeClipboardText } from "../clipboard";
import { ensureBlockId } from "../document";
import { captureBinding, stillBound } from "../binding";
import { pushToast } from "../toasts";

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
