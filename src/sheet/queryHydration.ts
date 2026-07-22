import { backend } from "../backend";
import { ensurePageLoaded, pageByName } from "../store";
import type { RefGroup } from "../types";
import { graphEpoch, graphMeta } from "../ui";

export const SHEET_RENDER_PAGE = 200;
const HYDRATE_CONCURRENCY = 4;
// Normal operation keeps at most 8 physical reads alive across the current and
// detached graph scopes. If all 8 belong to stale scopes and remain hung, the
// newest graph may borrow one final 4-read tranche after a short grace period.
// If twelve stale calls themselves hang, their 2s leases expire into a counted
// quarantine and only the latest graph may borrow one FINAL tranche, up to the
// absolute ceiling of sixteen. Further switches queue (and automatically retry
// when any physical invocation settles) rather than leaking per-switch tranches.
const HYDRATE_GLOBAL_SOFT_LIMIT = 8;
const HYDRATE_GLOBAL_ABSOLUTE_LIMIT = 12;
const HYDRATE_STALE_GRACE_MS = 250;
const HYDRATE_EXPIRED_ABSOLUTE_LIMIT = 16;
const HYDRATE_STALE_LEASE_MS = 2_000;
const HYDRATE_CIRCUIT_BACKOFF_MS = 1_000;
let activeHydrations = 0;
interface QueuedHydration {
  scope: string;
  start: () => void;
  cancel: () => void;
  counted: boolean;
  stale: boolean;
  expired: boolean;
  leaseTimer: ReturnType<typeof setTimeout> | null;
}
const hydrationQueue: QueuedHydration[] = [];
// Unlike `activeHydrations` (current-scope slots), this set retains detached
// old-scope reads until their uncancellable IPC actually settles. It supplies a
// hard cap across rapid repeated graph switches.
const runningHydrationItems = new Set<QueuedHydration>();
const pageHydrations = new Map<string, Promise<void>>();
const hydrationClaims = new Map<string, string>();
let limiterScope = "";
let overflowScope: string | null = null;
let overflowTimer: ReturnType<typeof setTimeout> | null = null;
let expiredRecoveryScope: string | null = null;
let circuitOpen = false;
let circuitRetryAt = 0;
let circuitTimer: ReturnType<typeof setTimeout> | null = null;

export interface QueryHydrationCircuitStatus {
  state: "closed" | "recovering" | "open";
  scope: string;
  underlying: number;
  quarantined: number;
  queued: number;
  retryAt: number | null;
}

/** Observable diagnostics for the unavoidable limitation that JS/Tauri invoke
 * promises cannot be physically cancelled once started. `underlying` counts
 * those real calls; it never exceeds 16. */
export function queryHydrationCircuitStatus(): QueryHydrationCircuitStatus {
  const quarantined = [...runningHydrationItems].filter((item) => item.expired).length;
  return {
    state: circuitOpen ? "open" : quarantined > 0 || overflowScope === limiterScope ? "recovering" : "closed",
    scope: limiterScope,
    underlying: runningHydrationItems.size,
    quarantined,
    queued: hydrationQueue.length,
    retryAt: circuitOpen ? circuitRetryAt : null,
  };
}

/** Explicit retry for an open circuit. It succeeds only when a real underlying
 * slot has become available; otherwise returning false tells the caller that a
 * restart/settlement is required rather than silently leaking a 17th call. */
export function retryQueryHydrationCircuit(): boolean {
  if (!circuitOpen || runningHydrationItems.size >= HYDRATE_EXPIRED_ABSOLUTE_LIMIT) return false;
  if (circuitTimer !== null) clearTimeout(circuitTimer);
  circuitTimer = null;
  circuitOpen = false;
  circuitRetryAt = 0;
  expiredRecoveryScope = limiterScope;
  runNextHydration();
  return true;
}

function groupIdentity(group: Pick<RefGroup, "kind" | "page" | "path">): string {
  return `${group.kind}\0${group.page}\0${group.path ?? ""}`;
}

/** Resolve a rendered query row to its exact source identity. Page names alone
 *  are not identities: a graph may contain both `pages/Foo.md` and a journal
 *  whose title is `Foo`. Block ids let us select the matching RefGroup without
 *  silently hydrating the other kind. If the id itself is duplicated across
 *  page/journal twins, decline hydration rather than make the wrong file
 *  editable. */
function sourceGroupForRow(
  row: { id: string; page: string },
  groupsByPage: ReadonlyMap<string, readonly RefGroup[]>,
): RefGroup | null {
  const candidates = (groupsByPage.get(row.page) ?? []).filter((group) =>
    group.blocks.some((block) => block.id === row.id)
  );
  if (!candidates.length) return null;
  const identities = new Set(candidates.map(groupIdentity));
  return identities.size === 1 ? candidates[0] : null;
}

function runNextHydration() {
  if (circuitOpen) {
    if (runningHydrationItems.size >= HYDRATE_EXPIRED_ABSOLUTE_LIMIT) return;
    const wait = circuitRetryAt - Date.now();
    if (wait > 0) {
      if (circuitTimer === null) {
        circuitTimer = setTimeout(() => {
          circuitTimer = null;
          runNextHydration();
        }, wait);
      }
      return;
    }
    circuitOpen = false;
    circuitRetryAt = 0;
    expiredRecoveryScope = limiterScope;
  }
  const physicalLimit = expiredRecoveryScope === limiterScope
    ? HYDRATE_EXPIRED_ABSOLUTE_LIMIT
    : overflowScope === limiterScope
      ? HYDRATE_GLOBAL_ABSOLUTE_LIMIT
      : HYDRATE_GLOBAL_SOFT_LIMIT;
  while (
    activeHydrations < HYDRATE_CONCURRENCY &&
    runningHydrationItems.size < physicalLimit &&
    hydrationQueue.length
  ) {
    const item = hydrationQueue.shift()!;
    activeHydrations++;
    item.counted = true;
    runningHydrationItems.add(item);
    item.start();
  }
  if (
    hydrationQueue.length &&
    runningHydrationItems.size >= HYDRATE_EXPIRED_ABSOLUTE_LIMIT
  ) {
    circuitOpen = true;
    circuitRetryAt = Date.now() + HYDRATE_CIRCUIT_BACKOFF_MS;
    return;
  }
  if (!hydrationQueue.length) {
    if (overflowTimer !== null) clearTimeout(overflowTimer);
    overflowTimer = null;
    return;
  }
  // The latest graph is blocked solely by stale physical calls at the normal
  // ceiling. Give ordinary rapid switches a grace window to settle naturally;
  // if they remain hung, open only the bounded 8→12 recovery tranche.
  if (
    overflowScope !== limiterScope &&
    overflowTimer === null &&
    runningHydrationItems.size >= HYDRATE_GLOBAL_SOFT_LIMIT &&
    runningHydrationItems.size < HYDRATE_GLOBAL_ABSOLUTE_LIMIT
  ) {
    const scope = limiterScope;
    overflowTimer = setTimeout(() => {
      overflowTimer = null;
      if (limiterScope !== scope || !hydrationQueue.length) return;
      overflowScope = scope;
      runNextHydration();
    }, HYDRATE_STALE_GRACE_MS);
  }
}

function releaseActiveSlot(item: QueuedHydration): void {
  if (!item.counted) return;
  item.counted = false;
  activeHydrations--;
}

function finishRunningHydration(item: QueuedHydration): void {
  if (item.leaseTimer !== null) clearTimeout(item.leaseTimer);
  item.leaseTimer = null;
  releaseActiveSlot(item);
  runningHydrationItems.delete(item);
}

function markStale(item: QueuedHydration): void {
  if (item.stale) return;
  item.stale = true;
  item.leaseTimer = setTimeout(() => {
    item.leaseTimer = null;
    if (!runningHydrationItems.has(item)) return;
    item.expired = true;
    // Only the latest graph receives the expired-call recovery tranche. Further
    // switches replace this scope, while the hard 16-call ceiling remains global.
    expiredRecoveryScope = limiterScope;
    runNextHydration();
  }, HYDRATE_STALE_LEASE_MS);
}

function enterLimiterScope(scope: string): void {
  if (scope === limiterScope) return;
  if (overflowTimer !== null) clearTimeout(overflowTimer);
  overflowTimer = null;
  overflowScope = null;
  expiredRecoveryScope = null;
  limiterScope = scope;
  // Active reads cannot be cancelled, but every active task performs the same
  // graph check after its await. Detach them from limiter accounting now so a
  // slow old read cannot occupy all four NEW-graph slots indefinitely. Queued
  // reads have not touched IPC yet and can be resolved immediately.
  const stale = hydrationQueue.splice(0);
  for (const item of stale) item.cancel();
  for (const item of runningHydrationItems) {
    if (item.scope === scope) continue;
    markStale(item);
    releaseActiveSlot(item);
    item.cancel();
  }
  runNextHydration();
}

function limited(scope: string, task: () => Promise<void>): Promise<void> {
  enterLimiterScope(scope);
  return new Promise<void>((resolve) => {
    let settled = false;
    const finish = () => {
      if (settled) return;
      settled = true;
      resolve();
    };
    const item: QueuedHydration = {
      scope,
      start: () => {
        void task().catch(() => {}).finally(() => {
          finishRunningHydration(item);
          finish();
          runNextHydration();
        });
      },
      cancel: finish,
      counted: false,
      stale: false,
      expired: false,
      leaseTimer: null,
    };
    hydrationQueue.push(item);
    runNextHydration();
  });
}

function sameGraph(root: string, epoch: number): boolean {
  return graphEpoch() === epoch && (graphMeta()?.root ?? "") === root;
}

function releaseClaim(claimKey: string, identity: string): void {
  if (hydrationClaims.get(claimKey) === identity) hydrationClaims.delete(claimKey);
}

/** Hydrate only pages represented in the currently rendered query-result window.
 * Query DTOs are sufficient for read-only display; full pages are needed only to
 * enable editing. A small worker pool prevents an IPC/file-load stampede. */
export async function hydrateVisibleQueryPages(
  rows: readonly { id: string; page: string }[],
  groups: readonly RefGroup[] | undefined,
): Promise<void> {
  const groupsByPage = new Map<string, RefGroup[]>();
  for (const group of groups ?? []) {
    const pageGroups = groupsByPage.get(group.page) ?? [];
    pageGroups.push(group);
    groupsByPage.set(group.page, pageGroups);
  }

  // First resolve every visible row by composite identity. If both a page and a
  // journal with the same name are visible, the current working set cannot hold
  // them simultaneously (it is keyed by page name), so keep both DTO-only and
  // read-only instead of racing which file becomes the editable one.
  const requested = new Map<string, RefGroup>();
  for (const row of rows) {
    const group = sourceGroupForRow(row, groupsByPage);
    if (group) requested.set(groupIdentity(group), group);
  }
  const kindsByName = new Map<string, Set<RefGroup["kind"]>>();
  for (const group of requested.values()) {
    const kinds = kindsByName.get(group.page) ?? new Set<RefGroup["kind"]>();
    kinds.add(group.kind);
    kindsByName.set(group.page, kinds);
  }

  const pending = [...requested.values()].filter((group) => {
    if ((kindsByName.get(group.page)?.size ?? 0) > 1) return false;
    // A differently-kinded same-name page already occupies the name-keyed store.
    // ensurePageLoaded cannot install this group safely without replacing it.
    const loaded = pageByName(group.page);
    return !loaded || (!!group.path && loaded.path !== group.path);
  });
  await Promise.all(pending.map((group) => {
    const epoch = graphEpoch();
    const root = graphMeta()?.root ?? "";
    const scope = `${root}\0${epoch}`;
    const identity = groupIdentity(group);
    const key = `${scope}\0${identity}`;
    const existing = pageHydrations.get(key);
    if (existing) return existing;

    // The store can hold only one kind for a display name. Serialize that claim
    // globally across separate SheetTable/SheetBoard invocations; a concurrent
    // opposite-kind request remains DTO-only instead of racing for the slot.
    const claimKey = `${scope}\0${group.page}`;
    const claim = hydrationClaims.get(claimKey);
    if (claim && claim !== identity) return Promise.resolve();
    hydrationClaims.set(claimKey, identity);

    const job = limited(scope, async () => {
      // A stale queued task must die before IPC, not merely discard afterward.
      if (!sameGraph(root, epoch)) return;
      const occupied = pageByName(group.page);
      if (occupied && occupied.kind === group.kind && (!group.path || occupied.path === group.path)) return;
      const dto = group.path
        ? await backend().getPageByPath(group.path)
        : await backend().getPage(group.page, group.kind);
      if (!sameGraph(root, epoch)) return;
      // Recheck occupancy after the await: another surface may have loaded a
      // same-name twin meanwhile. Never replace or alias that identity.
      const after = pageByName(group.page);
      if (after && (!group.path || after.path === group.path)) return;
      if (!dto || dto.name !== group.page || dto.kind !== group.kind || (group.path && dto.path !== group.path)) return;
      ensurePageLoaded(dto);
    }).finally(() => {
      pageHydrations.delete(key);
      releaseClaim(claimKey, identity);
    });
    pageHydrations.set(key, job);
    return job;
  }));
}
