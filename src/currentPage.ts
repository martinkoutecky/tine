import { backend } from "./backend";
import { captureBinding } from "./binding";
import { blockLink, isTineLinkId, pageLink } from "./deepLinks";
import { editingId } from "./editorController";
import type { CurrentPage } from "./nativeTineLinks";
import { ownedWhen, readOwned, readOwnedResource } from "./owned";
import { focusedRouterOf } from "./panes";
import { activeElement, activeWindowId } from "./windowRealm";

/** Snapshot the most recently focused graph window's active pane, whether or not
 * Tine has OS focus now (the asking script's app usually has it). O(route/URL bytes + DOM depth),
 * no graph I/O, mutation or rendering. The rendered editor supplies the existing
 * external block identity; another pane's retained edit is not focus. */
export function currentPage(): CurrentPage {
  const router = focusedRouterOf(activeWindowId());
  const route = router.route();
  if (route.kind !== "page" && route.kind !== "journals") return { kind: "no-current-page", reason: "not-page" };
  const element = activeElement();
  const block = element?.closest("[data-block-id]");
  const id = block?.getAttribute("data-block-id");
  const editing = id && id === editingId()
    && element?.closest("[data-pane-id]")?.getAttribute("data-pane-id") === router.paneId;
  const blockId = editing ? block?.getAttribute("data-block-ref") : null;
  const name = route.kind === "page" ? route.name
    : editing ? element?.closest("[data-page-name]")?.getAttribute("data-page-name") : null;
  if (!name) return { kind: "no-current-page", reason: "journals" };
  const block_url = blockId && isTineLinkId(blockId) ? blockLink(blockId) : null;
  return { kind: "current-page", name, url: pageLink(name), block_url };
}

/** One frontend answer per current binding request. Subscription belongs to
 * App's mount and is disposed even if registration completes after unmount. */
export function subscribeCurrentPage(): () => void {
  const api = backend().tineLinks;
  let alive = true;
  let stop = () => {};
  const owner = ownedWhen(() => alive);
  if (api?.onCurrentPageRequest) void readOwnedResource(owner, api.onCurrentPageRequest((request) => {
    if (!alive || request.binding_generation !== captureBinding().backendGeneration) return;
    if (api.currentPageAnswer) void readOwned(owner, api.currentPageAnswer(request.id, currentPage(), request.binding_generation))
      .catch(() => console.error("current page answer failed"));
  }), (dispose) => dispose()).then((result) => {
    if (result.kind === "current") {
      if (alive) stop = result.value;
      else result.value();
    }
  })
    .catch(() => console.error("current page subscription failed"));
  return () => { alive = false; stop(); };
}
