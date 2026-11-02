import { VIEW_KINDS } from "./editor/queryIr";
import { normalizeFriendlyPageMatchScope, normalizeQueryDisplayDraft } from "./editor/queryDisplayDraft";
import type { QueryPresentation, QueryRoute } from "./routeTypes";

// The pure half of editing a query tab's route (split from router.ts along its seam: it
// reads no router state). `router.ts` re-exports these names.

/** The one reader of a query presentation; null is "unreadable". */
export function normalizeQueryPresentation(value: unknown): QueryPresentation | null {
  return typeof value === "string" && (VIEW_KINDS as readonly string[]).includes(value) ? value as QueryPresentation : null;
}

/** What one atomic edit to the active query workspace may change. For each display
 * or presentation field: absent keeps the route's value, an explicit `undefined`
 * removes the override, a present value must normalize (`{}` stays an explicit clear). */
export type QueryRoutePatch = Partial<Pick<QueryRoute,
  "source" | "sourceKind" | "presentation" | "pageMatchScope" | "pagePresentation" | "blockPresentation" | "pageDisplay" | "blockDisplay">>;

/** The pure half of `updateActiveQuery`: the next route, or null when the patch
 * carries a value this build cannot read and the whole edit is refused. */
export function applyQueryRoutePatch(current: QueryRoute, patch: QueryRoutePatch): QueryRoute | null {
  const next: QueryRoute = { ...current };
  if (Object.hasOwn(patch, "source")) {
    if (typeof patch.source !== "string") return null;
    next.source = patch.source;
  }
  if (Object.hasOwn(patch, "sourceKind")) {
    if (patch.sourceKind !== "search" && patch.sourceKind !== "dsl") return null;
    next.sourceKind = patch.sourceKind;
  }
  if (Object.hasOwn(patch, "presentation")) {
    const presentation = normalizeQueryPresentation(patch.presentation);
    if (!presentation) return null;
    next.presentation = presentation;
  }
  for (const key of ["pageDisplay", "blockDisplay"] as const) {
    if (!Object.hasOwn(patch, key)) continue;
    if (patch[key] === undefined) { delete next[key]; continue; }
    const display = normalizeQueryDisplayDraft(patch[key]);
    if (!display) return null;
    next[key] = display;
  }
  for (const key of ["pagePresentation", "blockPresentation"] as const) {
    if (!Object.hasOwn(patch, key)) continue;
    if (patch[key] === undefined) { delete next[key]; continue; }
    const presentation = normalizeQueryPresentation(patch[key]);
    if (!presentation) return null;
    next[key] = presentation;
  }
  if (Object.hasOwn(patch, "pageMatchScope")) {
    if (patch.pageMatchScope === undefined) delete next.pageMatchScope;
    else {
      const scope = normalizeFriendlyPageMatchScope(patch.pageMatchScope);
      if (!scope) return null;
      next.pageMatchScope = scope;
    }
  }
  return next;
}
