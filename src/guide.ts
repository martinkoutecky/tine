import { backend } from "./backend";
import { bindingOwner, graphOwner, readOwned, writeOwned, type Owner, type WriteOwner } from "./owned";
import { focusedSurfaceOwner } from "./focusedSurface";
import { openPage, openPageInNewTab } from "./router";
import { loadGuidePages, pageByName } from "./document";
import { bumpPageInventoryRev, graphMeta, setGraphMeta } from "./graphSession";
import { pushToast } from "./toasts";
import type { GuidePage } from "./types";

export const GUIDE_DISPLAY_PREFIX = "Tine-guide/";
export const GUIDE_COPY_PREFIX = "tine-guide/";
export const GUIDE_INDEX_TITLE = "Tine Guide";

let guideLoad: Promise<GuidePage[]> | null = null;
let guideLoadOwner: Owner | null = null;
const guideTitles = new Map<string, string>();
const announcementShownForRoot = new Set<string>();

function key(name: string): string {
  return name.trim().toLowerCase();
}

export function guidePageName(title: string): string {
  return `${GUIDE_DISPLAY_PREFIX}${title}`;
}

export function isGuidePageName(name: string | undefined | null): boolean {
  return !!name && name.startsWith(GUIDE_DISPLAY_PREFIX);
}

export function guideTitleFromName(name: string): string {
  return isGuidePageName(name) ? name.slice(GUIDE_DISPLAY_PREFIX.length) : name;
}

export function guideTargetForLink(target: string, sourcePage?: string): string {
  if (!isGuidePageName(sourcePage)) return target;
  const title = guideTitles.get(key(target));
  return title ? guidePageName(title) : target;
}

/** Load bundled Guide templates into the read-only working set and return
 * their pages. Reuse a current cached promise unless force is true; force
 * starts a new load, but an earlier same-graph completion can still land.
 * A failed cached promise keeps rejecting until a forced load replaces it.
 * A stale completion returns []. Cost follows Guide page count/content. */
export async function ensureGuidePagesLoaded(force = false): Promise<GuidePage[]> {
  if (!force && guideLoad && guideLoadOwner?.()) return guideLoad;
  const owner = graphOwner();
  const pending = readOwned(owner, backend().guidePages())
    .then((result) => {
      if (result.kind === "stale") {
        if (guideLoad === pending) { guideLoad = null; guideLoadOwner = null; }
        return [];
      }
      const pages = result.value;
      guideTitles.clear();
      loadGuidePages(
        pages.map((g) => {
          guideTitles.set(key(g.title), g.title);
          return {
            ...g.page,
            name: guidePageName(g.title),
            title: g.title,
            read_only: true,
            guide: true,
          };
        })
      );
      return pages;
    });
  guideLoad = pending;
  guideLoadOwner = owner;
  return pending;
}

export async function openGuide(): Promise<void> {
  // The new tab opens in the surface the user asked from: if they moved to
  // another pane, tab or route while the Guide loaded, it must not appear there
  // (I-20). A failure still reports while the graph itself is current.
  const owner = focusedSurfaceOwner();
  const graphWhenAsked = graphOwner();
  try {
    await ensureGuidePagesLoaded(true);
    if (!owner()) return;
    openPageInNewTab(guidePageName(GUIDE_INDEX_TITLE), "page", undefined, true);
  } catch (e) {
    if (graphWhenAsked()) pushToast(`Couldn't open the Guide. (${String(e)})`, "error");
  }
}

/** Ask the backend to copy a Guide page into tine-guide/ and open the resulting
 * graph page. An existing copy is opened rather than created. Errors are
 * displayed as toasts and this function still resolves, so completion does
 * not certify that a copy exists. Cost follows copied Guide pages/assets. */
export async function copyGuideIntoGraph(pageName: string): Promise<void> {
  // The graph owner gates the bookkeeping and the success toast (the copy exists
  // in that graph either way); the surface owner gates only the navigation.
  const owner = bindingOwner();
  const surface = focusedSurfaceOwner();
  const page = pageByName(pageName);
  const title = guideTitleFromName(page?.name ?? pageName);
  try {
    const copied = await writeOwned(owner, backend().copyGuideIntoGraph(title, "replace-page"));
    if (copied.kind === "stale") return;
    const result = copied.value;
    if ((result.created_pages?.length ?? 0) > 0) bumpPageInventoryRev();
    pushToast(
      result.created
        ? "Copied the guide into your graph under tine-guide/."
        : "The guide is already in your graph - opened it.",
      "success"
    );
    if (surface()) openPage(result.name, "page");
  } catch (e) {
    pushToast(`Couldn't copy the Guide into your graph. (${String(e)})`, "error");
  }
}

function markGuideAnnounced(owner: WriteOwner) {
  if (!owner()) return;
  const meta = graphMeta();
  if (meta && !meta.guide_announced) {
    setGraphMeta({ ...meta, guide_announced: true });
  }
  void writeOwned(owner, backend().setGuideAnnounced(true)).catch(() => {
    const current = graphMeta();
    if (current && current.root === meta?.root && current.guide_announced) {
      setGraphMeta({ ...current, guide_announced: false });
    }
    pushToast("Could not save Guide announcement preference.", "error");
  });
}

/** Show a sticky Guide toast once per graph root in this process when metadata
 * says it is unannounced. Dismissal persists guide_announced; write failure
 * restores the flag and toasts. O(1) plus deferred backend write. */
export function maybeShowGuideAnnouncement() {
  const meta = graphMeta();
  if (!meta || meta.guide_announced || announcementShownForRoot.has(meta.root)) return;
  announcementShownForRoot.add(meta.root);
  const owner = bindingOwner();
  pushToast("New: in-app Guide \u2014 learn Sheets, formulas & queries.", "info", {
    sticky: true,
    action: {
      label: "Open Guide",
      run: () => void openGuide(),
    },
    onDismiss: () => markGuideAnnounced(owner),
  });
}
