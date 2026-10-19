// The share dialog's state (docs/contracts/export-definitions.md, SHARE-DIALOG.md §3): what it opened on, and the
// hook a folder export runs once it has written its folder. Kept out of ui.ts (production files may not grow).
import { graphScopedSignal } from "./binding";
import type { QueryPublicationRequest } from "./types";
import type { ShareDefinitionTarget } from "./quickShare/definitions";
import { openQueryExport } from "./ui";

/** What the share dialog shares: the current page, or an existing query block (`macro` is its `{{…}}` text;
 *  `query.hostBlockId` is the block, so the destination property goes on it). */
export type ShareTarget =
  | { page: string }
  | { query: QueryPublicationRequest; macro: string };
/** One graph-bound share dialog; a graph switch closes it. */
export const [shareTarget, setShareTarget] = graphScopedSignal<ShareTarget>();
export function openShareDialog(target: ShareTarget): void { setShareTarget(target); }
export function closeShareDialog(): void { setShareTarget(null); }

/** What a query export does once it has written its folder (the share dialog saves its definition then). */
export type QueryExportAfter = (done: { folder: string }) => void | Promise<void>;
// Keyed by the request object the export dialog is open on: a dismissed or replaced request takes its hook with it.
const exportAfter = new WeakMap<QueryPublicationRequest, QueryExportAfter>();
export function queryExportAfter(request: QueryPublicationRequest): QueryExportAfter | undefined { return exportAfter.get(request); }
/** The share dialog hands its reviewed request to the folder export and saves the definition once it landed. */
export function openQueryExportThen(request: QueryPublicationRequest, after: QueryExportAfter): void {
  exportAfter.set(request, after);
  openQueryExport(request);
}

/** The quick share dialog (DESIGN §3.9): the reviewed request the share dialog's Link option hands over. Graph-bound
 *  like the share dialog; closing it discards the prepared payload. A created share is recorded in the graph at
 *  `definition` (the query block it started from, or the exports page). */
export const [quickShareRequest, setQuickShareRequest] = graphScopedSignal<QueryPublicationRequest>();
// Where a created share's `tine.share::` goes (D-20261018-Q1), keyed by the request like the folder hook.
const shareDefinitions = new WeakMap<QueryPublicationRequest, ShareDefinitionTarget>();
export function quickShareDefinition(request: QueryPublicationRequest): ShareDefinitionTarget | undefined { return shareDefinitions.get(request); }
export function openQuickShare(request: QueryPublicationRequest, definition?: ShareDefinitionTarget): void {
  if (definition) shareDefinitions.set(request, definition);
  setQuickShareRequest(request);
}
export function closeQuickShare(): void { setQuickShareRequest(null); }
