import { msg } from "./i18n";
import { backend } from "./backend";
import { ensurePageLoaded, pageByName, reportPageLoadRefusal } from "./document";
import { graphOwner, readOwned } from "./owned";
import { pushToast } from "./toasts";
import type { PageKind } from "./types";

export interface SourcePage {
  page: string;
  pageKind: PageKind;
  /** Exact file, when the surface knows it. */
  path?: string;
}

/** Bring the page a reference/query surface shows a block of into the working
 * set, so an edit made there runs through that page's ordinary edit path
 * (GH #518, #486). A read only: `ensurePageLoaded` never replaces a loaded
 * page's unsaved edits, and a refusal or missing page is reported visibly with
 * `notDone` ("The task was not changed."). Resolves true when the exact page
 * is loaded. Cost: one backend page read when it is not loaded yet, plus
 * O(page blocks) to admit it. A graph switch during the read resolves false. */
export async function loadSourcePage(source: SourcePage, notDone: string): Promise<boolean> {
  const exact = () => {
    const loaded = pageByName(source.page);
    return !!loaded && loaded.kind === source.pageKind && (source.path === undefined || loaded.id === source.path);
  };
  if (exact()) return true;
  const result = await readOwned(graphOwner(), source.path ? backend().getPageByPath(source.path) : backend().getPage(source.page, source.pageKind));
  if (result.kind === "stale") return false;
  if (!result.value) {
    pushToast(msg("source-page-is-no-longer-there", { page: String(source.page), not_done: String(notDone) }), "error");
    return false;
  }
  const refusal = ensurePageLoaded(result.value);
  if (refusal) {
    reportPageLoadRefusal(refusal, notDone);
    return false;
  }
  return exact();
}
