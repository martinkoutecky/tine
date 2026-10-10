import type { ClipboardSourcePage } from "../clipboard";
import type { PageKind } from "../types";

/** Does a cut grant's source still name the page instance it was taken from
 *  (name, kind, file, instance generation)? Shared by the save engine and the
 *  page-host client (step 3b R7), which adds its key and session checks. */
export function cutSourceMatches(
  expected: ClipboardSourcePage,
  page: { name: string; kind: PageKind; id?: string | null } | undefined,
  generation: number | null,
): boolean {
  return !!page
    && page.name === expected.name
    && page.kind === expected.kind
    // A grant taken before the page's first save has no file id; that save
    // then records the id it created (`setPageId`) on the same instance, so
    // the generation check alone pins it.
    && (expected.path === undefined || page.id === expected.path)
    && generation === expected.generation;
}
