import { createSignal, onCleanup } from "solid-js";
import { t } from "../i18n";
import { backend } from "../backend";
import { node, seenPageSnapshot } from "../document";
import { touchPrimary } from "../inputCapability";
import { bindingIdentity } from "../binding";
import { graphOwner, readOwned } from "../owned";
import { dbg } from "../debug";
import { reportUiFailure } from "../uiFailure";
import { nodeAtPath } from "./marks";
import type { BlockLastChanged } from "../backendTypes";
import "../styles/bulletHistory.css";

let reportedBinding: string | null = null;
/** A failed history read is shown once per graph binding (GH #980): hovering
 * the next bullet repeats the same cause, so the later failures only go to the
 * opt-in debug log. Absent history (no git, no repository, an untracked page)
 * is not a failure and never arrives here. O(1). */
export function reportBlockHistoryFailure(error: unknown): void {
  const binding = bindingIdentity();
  if (reportedBinding === binding) { dbg(`block-history (already reported): ${String(error)}`); return; }
  reportedBinding = binding;
  reportUiFailure("block-history", error);
}

/** Add git history to an existing bullet title, in live and fallback rows.
 * One page snapshot/IPC on mouse enter; no reads during render. A fallback
 * without a loaded current source says Unknown rather than dating a DTO copy. */
export function createBulletHistory(id: () => string | null, help: () => string) {
  // Only the hover's conditional tooltip reads the date. Resting bullets keep
  // just their hover signal; allocate the answer signal before showing it.
  let changed: ReturnType<typeof createSignal<BlockLastChanged | null>> | undefined;
  const [visible, setVisible] = createSignal(false);
  let sequence = 0;
  onCleanup(() => sequence++);
  const enter = () => {
    if (touchPrimary()) return;
    changed ??= createSignal<BlockLastChanged | null>(null);
    const setChanged = changed[1];
    setChanged(null);
    setVisible(true);
    const token = ++sequence;
    const sourceId = id();
    const source = sourceId ? node(sourceId) : null;
    const snapshot = source && seenPageSnapshot(source.page);
    if (!sourceId || !source || !snapshot) { setChanged({ kind: "unknown" }); return; }
    const raw = source.raw;
    const page = source.page;
    const owner = graphOwner(() => token === sequence && snapshot.sameOutline() && id() === sourceId && node(sourceId)?.raw === raw && node(sourceId)?.page === page);
    void readOwned(owner, backend().getPageLastChanged(snapshot.path, snapshot.text)).then(answer => {
      if (answer.kind === "stale") return;
      const at = answer.value.find(row => nodeAtPath(snapshot.roots, row.block_path)?.id === sourceId);
      setChanged(at?.changed ?? { kind: "unknown" });
    }, error => { if (owner()) reportBlockHistoryFailure(error); });
  };
  const text = () => {
    const date = changed?.[0]();
    if (!date) return help();
    const info = date.kind === "committed" ? t("block-last-changed-date", { date: new Date(date.time * 1000).toLocaleString() })
      : date.kind === "uncommitted" ? t("block-last-changed-uncommitted")
      : date.kind === "history_limit" ? t("block-last-changed-limit") : t("block-last-changed-unknown");
    return `${info}\n${help()}`;
  };
  return { text, visible: () => visible() && !touchPrimary(), nativeTitle: () => touchPrimary() ? help() : undefined,
    enter, leave: () => { sequence++; setVisible(false); } };
}
