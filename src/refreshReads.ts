// GH #746 (related): the whole-graph reads that `dataRev` readers issue
// (backlinks, unlinked refs, page icons, block refs, queries, journal views).
// The host runs each on its blocking pool and never cancels one, so a refresh
// round that starts while the previous round's reads still run competes with
// them for CPU. The save-driven `dataRev` bump (document/host/wiring.ts) waits
// here until none is in flight. Frontend-only bookkeeping: no IPC, no persisted
// state.

/** The `dataRev` readers' read commands (backend.ts counts each call). A
 * closed list: a command left out only loses the wait, never a refresh. Page
 * loads (`get_page`) and the search panel (`search`) are not refresh reads. */
export const REFRESH_READ_COMMANDS: ReadonlySet<string> = new Set([
  "get_backlinks", "get_unlinked_refs", "page_icons", "resolve_blocks", "preview_block", "query_parse", "query_run",
  "query_registry", "run_graph_search", "journal_feed_page", "journal_content_days", "list_templates",
  "block_referrers", "get_backlink_filter_context", "page_backlink_counts", "page_modified",
]);

let inFlight = 0;
let generation = 0;
let idle: (() => void)[] = [];

function settled(owner: number): void {
  if (owner !== generation || --inFlight > 0) return;
  const waiters = idle;
  idle = [];
  for (const wake of waiters) wake();
}

/** Count `read` in flight until it settles, answered or failed. Returns `read`. */
export function trackRefreshRead<T>(read: Promise<T>): Promise<T> {
  const owner = generation;
  inFlight += 1;
  read.then(() => settled(owner), () => settled(owner));
  return read;
}

/** Resolves once no refresh read is in flight (at once when none is). */
export function refreshReadsIdle(): Promise<void> {
  return inFlight === 0 ? Promise.resolve() : new Promise((resolve) => { idle.push(resolve); });
}

export function refreshReadsInFlight(): number {
  return inFlight;
}

/** The binding ended (graph switch, restore): the old graph's reads no longer
 * hold anything; their late settlements are ignored. */
export function resetRefreshReads(): void {
  generation += 1;
  inFlight = 0;
  const waiters = idle;
  idle = [];
  for (const wake of waiters) wake();
}
