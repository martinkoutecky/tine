// Entry points of "changed since you last looked" (ADR 0073): the palette
// commands, which act on the focused pane's routed page, and the Page actions
// menu items, the quiet affordance for one page.
import { focusedRouter } from "../panes";
import { loadedPage } from "../document";
import { forgetPageSeen, markPageSeen, seenBaselineFor, seenSupported } from "./baseline";

/** The focused pane's routed page, when it can be tracked. */
function focusedSeenPage(): string | null {
  const route = focusedRouter().route();
  if (route.kind !== "page") return null;
  const page = loadedPage(route.name);
  return page && !page.guide && seenSupported() ? page.name : null;
}

export const seenCommands = {
  markAvailable: () => focusedSeenPage() !== null,
  mark: () => { const name = focusedSeenPage(); if (name) void markPageSeen(name); },
  forgetAvailable: () => { const name = focusedSeenPage(); return !!name && !!seenBaselineFor(name); },
  forget: () => { const name = focusedSeenPage(); if (name) void forgetPageSeen(name); },
};

/** Page actions items for `name`: Mark page seen, and Forget seen state while tracked. */
export function seenPageMenuItems(name: string): { id: string; label: string; run: () => void }[] {
  const page = loadedPage(name);
  if (!page || page.guide || !seenSupported()) return [];
  const items = [{ id: "seen-mark", label: "Mark page seen", run: () => void markPageSeen(name) }];
  if (seenBaselineFor(name)) items.push({ id: "seen-forget", label: "Forget seen state", run: () => void forgetPageSeen(name) });
  return items;
}
