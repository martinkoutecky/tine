/** External navigation over the existing backend, graph-switch and pane route
 * doors. Scans only on explicit opening and never writes; Copy link (which may
 * stamp identity) lives in components/blockLinkCopy.ts with the other reference creators.
 * A stale graph binding cancels outstanding resolution/copy. Errors are shown,
 * with no creation or current-graph fallback. Native queue installation returns
 * an idempotent disposer and drains again after subscription (cold/warm race). */
import { ownedWhen, readOwned, writeOwned, readOwnedResource } from "./owned";
import { backend, isTauri } from "./backend";
import { captureBinding, stillBound } from "./binding";
import { loadGraphPath } from "./graph";
import { graphMeta } from "./graphSession";
import { focusedRouter, focusedPaneId } from "./panes";
import { pushToast } from "./toasts";
import { parseTineLink, parseAppRoute, type AppRoute } from "./deepLinks";
import { admitPageFile, captureEmptyPage, pageByName, reportPageLoadRefusal } from "./document";
import { journalTitle, appNow } from "./journal";
import { openSwitcher } from "./ui";
import { focusPageTrailing } from "./components/pageTrailing";
import { resolvedTarget, refreshPageIndex } from "./pageIndex";
import { chooseLinkGraph } from "./components/DeepLinkGraphChoice";

export interface LinkTarget {
  root: string;
  graphId?: string | null;
  name?: string | null;
  pageKind?: "page" | "journal" | null;
  path?: string | null;
  block?: string | null;
  error?: string | null;
}
export type LinkDelivery = { kind: "url"; url: string } | { kind: "target"; target: LinkTarget };

/** Resolve one URL and open its existing destination in the focused pane.
 * O(known graph paths/pages/blocks), native scans off the UI thread. A copy
 * choice is saved in existing device settings and reused only while present. */
export async function openTineLink(delivery: LinkDelivery, alive: () => boolean = () => true): Promise<void> {
  const binding = captureBinding();
  const current = () => alive() && stillBound(binding);
  const owner = ownedWhen(current);
  try {
    const api = backend();
    let target: LinkTarget;
    const app = delivery.kind === "url" ? parseAppRoute(delivery.url) : null;
    if (app) return await openAppRoute(app, current);
    if (delivery.kind === "url") {
      const request = parseTineLink(delivery.url);
      if (!api.tineLinks?.scanKnownGraphs) throw new Error("External links are available in the Tine app");
      const scanned = await readOwned(owner, api.tineLinks.scanKnownGraphs(request));
      if (scanned.kind === "stale") return;
      const candidates = scanned.value;
      if (!candidates.length) throw new Error("Tine link target not found in any known graph");
      target = candidates[0];
      if (candidates.length > 1) {
        const commonId = candidates[0].graphId;
        const graphId = commonId && candidates.every((candidate) => candidate.graphId === commonId) ? commonId : undefined;
        const key = `tine-link-choice:${request.graph ?? graphId ?? request.block}`;
        const remembered = await readOwned(owner, api.getAppString(key, ""));
        if (remembered.kind === "stale") return;
        const match = candidates.find((candidate) => candidate.root === remembered.value);
        if (match) target = match;
        else {
          const chosen = await chooseLinkGraph(candidates);
          if (!chosen || !current()) return;
          target = chosen;
          await writeOwned(owner, api.setAppString(key, target.root));
          if (!current()) return;
        }
      }
    } else target = delivery.target;
    if (target.error) throw new Error(target.error);
    if ((target.name || target.block) && !target.path)
      throw new Error("Tine link target not found in the chosen graph copy");
    const handoff = await writeOwned(owner, api.tineLinks?.handoff(target) ?? Promise.resolve(false));
    if (handoff.kind === "stale" || handoff.value) return;
    if (!current()) return;
    if (graphMeta()?.root !== target.root) {
      const result = await loadGraphPath(target.root);
      if (!alive() || (result.kind !== "loaded" && result.kind !== "already_current")) return;
    }
    // Resolve once more in the bound graph: deletion/move during selection or
    // opening must not route to a virtual missing page that typing could create.
    if (!target.name) return;
    const destinationBinding = captureBinding();
    const owned = () => alive() && stillBound(destinationBinding);
    const destinationOwner = ownedWhen(owned);
    const read = await readOwned(destinationOwner, target.path ? api.getPageByPath(target.path) : Promise.resolve(null));
    if (read.kind === "stale") return;
    const page = read.value;
    if (!page) throw new Error("The Tine link page no longer exists");
    if (target.block) {
      const blocks = await readOwned(destinationOwner, api.resolveBlocks([target.block]));
      if (blocks.kind === "stale") return;
      const found = blocks.value[0];
      if (!found || found.page !== page.name) throw new Error("The Tine link block moved or no longer exists; open the link again");
    }
    focusedRouter().openInNewTab({ kind: "page", name: page.name,
      pageKind: page.kind, path: page.id, ...(target.block ? { block: target.block } : {}) }, true);
  } catch (error) {
    if (alive()) pushToast(`Couldn't open Tine link: ${String(error)}`, "error");
  }
}

/** Open a current-graph route (ADR 0073) in the focused pane. Never creates
 * a page: an unknown page name is an error, and today's journal stays the
 * usual phantom day until the user types. O(1) backend reads. */
async function openAppRoute(app: AppRoute, current: () => boolean): Promise<void> {
  if (!graphMeta()) throw new Error("open a graph first");
  const owner = ownedWhen(current);
  const router = focusedRouter();
  if (app.route === "search") { openSwitcher(app.query ? { prefill: app.query } : undefined); return; }
  const today = journalTitle(appNow());
  if (app.route === "page") {
    // The one frontend name answerer (pageIndex.ts); a cold launch waits for it.
    const fileId = () => (["page", "journal"] as const).map((kind) => resolvedTarget(app.page, kind))
      .map((target) => target?.kind === "existing" ? target.id : target?.kind === "alias" ? target.owners[0] : null)
      .find((id) => !!id);
    if (!fileId()) await refreshPageIndex();
    if (!current()) return;
    const id = fileId();
    const read = id ? await readOwned(owner, backend().getPageByPath(id)) : null;
    if (read?.kind === "stale") return;
    if (!read?.value) throw new Error(`the page "${app.page}" doesn't exist in the open graph`);
    router.openInNewTab({ kind: "page", name: read.value.name, pageKind: read.value.kind, path: read.value.id }, true);
    return;
  }
  router.openPage(today, "journal");
  if (app.route === "today") return;
  // Quick capture: the same "continue writing below" as the page's trailing
  // target, in the pane that shows the journal.
  const admitted = await admitPageFile(today, "journal", owner, captureEmptyPage(today, "journal"));
  if (admitted && admitted !== "stale") reportPageLoadRefusal(admitted, "Nothing was opened for writing.");
  if (admitted || !owner()) return;
  const pane = focusedPaneId();
  focusPageTrailing(pageByName(today), pane === "main" ? "main" : `pane:${pane}`);
}

export async function installTineLinks(alive: () => boolean): Promise<() => void> {
  if (!isTauri() || !backend().tineLinks?.take) return () => {};
  let disposed = false;
  let serial = Promise.resolve();
  const owner = ownedWhen(() => !disposed && alive());
  const drain = () => {
    serial = serial.then(async () => {
      if (disposed || !alive()) return;
      const pending = await readOwned(owner, backend().tineLinks!.take());
      if (pending.kind === "stale") return;
      for (const delivery of pending.value) {
        if (disposed || !alive()) break;
        await openTineLink(delivery, () => !disposed && alive());
      }
    }).catch((error) => { if (!disposed && alive()) pushToast(`Couldn't receive Tine link: ${String(error)}`, "error"); });
  };
  let unlisten: () => void;
  try {
    const registration = await readOwnedResource(owner, backend().tineLinks!.subscribe(drain), (stop) => stop());
    if (registration.kind === "stale") return () => {};
    unlisten = registration.value;
  }
  catch (error) {
    if (alive()) pushToast(`Couldn't listen for Tine links: ${String(error)}`, "error");
    return () => {};
  }
  if (!alive()) { unlisten(); return () => {}; }
  drain();
  return () => { disposed = true; unlisten(); };
}
