import { createSignal } from "solid-js";
import { dirtyPages, savingPages, isDirty, isSaving, graphBinding } from "./persistence";
import { conflicts, conflictQueue } from "./ui";
import { pageToDto } from "./store";
import type { PageDto } from "./types";
import { exportOutline, DEFAULT_EXPORT_OPTIONS, type ExportNode } from "./editor/exportText";
import { onGraphRebound } from "./modeHooks";

const [recoveryBinding, setRecoveryBinding] = createSignal<number | null>(null);
export const unsavedRecoveryOpen = () => recoveryBinding() === graphBinding();
export const openUnsavedRecovery = () => setRecoveryBinding(graphBinding());
export const closeUnsavedRecovery = () => setRecoveryBinding(null);
onGraphRebound(closeUnsavedRecovery);

export function unsavedRecoveryPages() {
  const names = new Set([...dirtyPages(), ...savingPages(), ...conflicts()]);
  return [...names].map((name) => {
    const conflict = conflictQueue().find((c) => c.source === "live-save" && c.page_name === name);
    return {
      name,
      state: isSaving(name) ? "Saving" : conflicts().includes(name) ? "Conflict" : isDirty(name) ? "Not saved" : "Needs review",
      page: pageToDto(name),
      retained: conflict?.live?.page,
    };
  });
}

/** Preserve source markup/properties; recovery never expands queries or refs. */
export function recoveryDraftText(page: PageDto): string {
  const nodes = (blocks: PageDto["blocks"]): ExportNode[] => blocks.map((block) => ({
    raw: block.raw, format: page.format ?? "md", children: nodes(block.children),
  }));
  const outline = exportOutline(nodes(page.blocks), { ...DEFAULT_EXPORT_OPTIONS, content: "source" });
  return (page.pre_block === null ? "" : `${page.pre_block}\n`) + outline;
}
