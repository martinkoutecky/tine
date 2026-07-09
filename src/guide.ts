import { backend } from "./backend";
import { openPage, openPageInNewTab } from "./router";
import { loadGuidePages, pageByName } from "./store";
import { graphMeta, pushToast, setGraphMeta } from "./ui";
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
  guideLoad = backend()
    .guidePages()
    .then((pages) => {
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
  try {
    await ensureGuidePagesLoaded(true);
    openPageInNewTab(guidePageName(GUIDE_INDEX_TITLE), "page", undefined, true);
  } catch (e) {
    pushToast(`Couldn't open the Guide. (${String(e)})`, "error");
  }
}

export async function copyGuideIntoGraph(pageName: string): Promise<void> {
  const page = pageByName(pageName);
  const title = guideTitleFromName(page?.name ?? pageName);
  try {
    const result = await backend().copyGuideIntoGraph(title);
    pushToast(
      result.created
        ? "Copied the guide into your graph under tine-guide/."
        : "The guide is already in your graph - opened it.",
      "success"
    );
    openPage(result.name, "page");
  } catch (e) {
    pushToast(`Couldn't copy the Guide into your graph. (${String(e)})`, "error");
  }
}

function markGuideAnnounced() {
  const meta = graphMeta();
  if (meta && !meta.guide_announced) {
    setGraphMeta({ ...meta, guide_announced: true });
  }
  void backend().setGuideAnnounced(true).catch(() => {});
}

export function maybeShowGuideAnnouncement() {
  const meta = graphMeta();
  if (!meta || meta.guide_announced || announcementShownForRoot.has(meta.root)) return;
  announcementShownForRoot.add(meta.root);
  pushToast("New: in-app Guide \u2014 learn Sheets, formulas & queries.", "info", {
    sticky: true,
    action: {
      label: "Open Guide",
      run: () => void openGuide(),
    },
    onDismiss: markGuideAnnounced,
  });
}
