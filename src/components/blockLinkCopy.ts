// Edit-mode "copy a link to this block" commands: builtin Mod+C with no text
// selected copies `((uuid))` (OG parity), and Mod+Shift+C copies the embed
// `{{embed ((uuid))}}` (GH #279). Both persist the block's id:: through the
// ordinary guarded save before the link is handed out, so a pasted link never
// points at an id that exists only in memory.
import { writeClipboardText } from "../clipboard";
import { ensureBlockId } from "../document";
import { pushToast } from "../toasts";

export type BlockLinkKind = "ref" | "embed";

const TEXT: Record<BlockLinkKind, { wrap: (uuid: string) => string; ok: string; noun: string }> = {
  ref: { wrap: (uuid) => `((${uuid}))`, ok: "Copied block ref", noun: "reference" },
  embed: { wrap: (uuid) => `{{embed ((${uuid}))}}`, ok: "Copied block embed", noun: "embed" },
};

export function copyBlockLink(id: string, kind: BlockLinkKind): Promise<void> {
  const text = TEXT[kind];
  return ensureBlockId(id).then((uuid) => {
    if (!uuid) {
      pushToast(`Couldn't save the block id — ${text.noun} not copied.`, "error");
      return;
    }
    return writeClipboardText(text.wrap(uuid))
      .then(() => { pushToast(text.ok, "success"); })
      .catch(() => { pushToast(`Couldn't copy block ${kind}: clipboard write failed.`, "error"); });
  });
}
