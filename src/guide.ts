import { backend } from "./backend";
import { captureBinding, stillBound, type Binding } from "./binding";
import { openPage, openPageInNewTab } from "./router";
import { loadGuidePages, pageByName } from "./document";
import { bumpPageInventoryRev, graphMeta, setGraphMeta } from "./graphSession";
import { pushToast } from "./toasts";
import type { GuidePage } from "./types";

export const GUIDE_DISPLAY_PREFIX = "Tine-guide/";
export const GUIDE_COPY_PREFIX = "tine-guide/";
export const GUIDE_INDEX_TITLE = "Tine Guide";

let guideLoad: Promise<GuidePage[]> | null = null;
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

export async function ensureGuidePagesLoaded(force = false): Promise<GuidePage[]> {
  if (!force && guideLoad) return guideLoad;
  const binding = captureBinding();
  guideLoad = backend()
    .guidePages()
    .then((pages) => {
      if (!stillBound(binding)) return pages;
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
  return guideLoad;
}

export async function openGuide(): Promise<void> {
  const binding = captureBinding();
  try {
    await ensureGuidePagesLoaded(true);
    if (!stillBound(binding)) return;
    openPageInNewTab(guidePageName(GUIDE_INDEX_TITLE), "page", undefined, true);
  } catch (e) {
    if (stillBound(binding)) pushToast(`Couldn't open the Guide. (${String(e)})`, "error");
  }
}

export async function copyGuideIntoGraph(pageName: string): Promise<void> {
  const binding = captureBinding();
  const page = pageByName(pageName);
  const title = guideTitleFromName(page?.name ?? pageName);
  try {
    const result = await backend().copyGuideIntoGraph(title, "replace-page");
    if (!stillBound(binding)) return;
    if ((result.created_pages?.length ?? 0) > 0) bumpPageInventoryRev();
    pushToast(
      result.created
        ? "Copied the guide into your graph under tine-guide/."
        : "The guide is already in your graph - opened it.",
      "success"
    );
    openPage(result.name, "page");
  } catch (e) {
    if (stillBound(binding)) pushToast(`Couldn't copy the Guide into your graph. (${String(e)})`, "error");
  }
}

function markGuideAnnounced(binding: Binding) {
  if (!stillBound(binding)) return;
  const meta = graphMeta();
  if (meta && !meta.guide_announced) {
    setGraphMeta({ ...meta, guide_announced: true });
  }
  void backend().setGuideAnnounced(true).catch(() => {
    if (!stillBound(binding)) return;
    const current = graphMeta();
    if (current && current.root === meta?.root && current.guide_announced) {
      setGraphMeta({ ...current, guide_announced: false });
    }
    pushToast("Could not save Guide announcement preference.", "error");
  });
}

export function maybeShowGuideAnnouncement() {
  const meta = graphMeta();
  if (!meta || meta.guide_announced || announcementShownForRoot.has(meta.root)) return;
  announcementShownForRoot.add(meta.root);
  const binding = captureBinding();
  pushToast("New: in-app Guide \u2014 learn Sheets, formulas & queries.", "info", {
    sticky: true,
    action: {
      label: "Open Guide",
      run: () => void openGuide(),
    },
    onDismiss: () => markGuideAnnounced(binding),
  });
}
