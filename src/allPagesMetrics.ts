/** Lazy graph-scoped All Pages metrics. Panes share one seed per metric and
 * patch final publication rows. Only an in-flight seed retains replay rows.
 * Per-target revisions (deletions included) stay O(live rows): past
 * 2 x live + 64 retained revisions the metric reseeds, so a reseed costs one
 * metric read per O(live rows) changed targets. */
import { batch, createEffect, createRoot, createSignal, onCleanup } from "solid-js";
import { backend, type GraphAnswersChange } from "./backend";
import { graphEpoch } from "./graphSession";
import { graphOwner, readOwned } from "./owned";
import { observePageMetricsAnswers } from "./graphAnswers";
import { backlinkCountsByRow } from "./allPagesList";
import { reportUiFailure } from "./uiFailure";

type Rows = readonly (readonly [string, number | null])[];
type Delta = { rev: bigint; rows: Rows | null };

const shared = createRoot(() => {
  const [users, setUsers] = createSignal(0);
  let epoch = graphEpoch();
  function metric() {
    let values = new Map<string, number>();
    const [version, setVersion] = createSignal(0);
    let seedRev = -1n, attempted = false;
    const rowRevs = new Map<string, bigint>();
    let pending: { rows: Map<string, { rev: bigint; value: number | null }>; invalidRev: bigint } | null = null;
    /** Apply a publication; true when the retained revisions outgrew the
     * live rows and the metric must reseed. O(rows in the delta). */
    const apply = (delta: Delta): boolean => {
      if (!delta.rows || delta.rev <= seedRev) return false;
      let changed = false;
      for (const [key, value] of delta.rows) {
        if (delta.rev <= (rowRevs.get(key) ?? seedRev)) continue;
        rowRevs.set(key, delta.rev);
        if (value === null) changed = values.delete(key) || changed;
        else if (values.get(key) !== value) { values.set(key, value); changed = true; }
      }
      if (changed) setVersion(n => n + 1);
      return rowRevs.size > 2 * values.size + 64;
    };
    return {
      read: () => { version(); return values as ReadonlyMap<string, number>; },
      reset() { values.clear(); rowRevs.clear(); seedRev = -1n; attempted = false; pending = null; setVersion(n => n + 1); },
      /** Feed a publication; true means "reseed now" (full invalidation or
       * revision retention past its bound). */
      delta(delta: Delta) {
        if (pending) {
          if (delta.rows === null) pending.invalidRev = delta.rev > pending.invalidRev ? delta.rev : pending.invalidRev;
          else for (const [key, value] of delta.rows) {
            if ((pending.rows.get(key)?.rev ?? -1n) < delta.rev) pending.rows.set(key, { rev: delta.rev, value });
          }
        }
        else if (attempted) {
          if ((delta.rows === null && delta.rev > seedRev) || apply(delta)) {
            attempted = false; rowRevs.clear(); return true;
          }
        }
        return false;
      },
      async seed(load: () => Promise<{ rev: bigint; rows: Map<string, number> }>) {
        if (attempted) return;
        attempted = true;
        const binding = epoch;
        pending = { rows: new Map(), invalidRev: -1n };
        try {
          let result = await load();
          if (binding !== graphEpoch()) return;
          // A command can capture a snapshot before the lazy native owner was
          // armed. Retry only that cold race, while keeping the replay window.
          while (pending && pending.invalidRev > result.rev) {
            result = await load();
            if (binding !== graphEpoch()) return;
          }
          batch(() => {
            values = result.rows;
            seedRev = result.rev;
            rowRevs.clear();
            for (const [key, update] of pending?.rows ?? []) {
              if (update.rev <= result.rev) continue;
              rowRevs.set(key, update.rev);
              if (update.value === null) values.delete(key); else values.set(key, update.value);
            }
            pending = null;
            setVersion(n => n + 1);
          });
        } catch (error) {
          if (binding === graphEpoch()) { pending = null; attempted = false; reportUiFailure("page-inventory", error); }
        }
      },
    };
  }
  const modified = metric(), backlinks = metric();
  const reset = () => {
    if (epoch === graphEpoch()) return;
    epoch = graphEpoch(); modified.reset(); backlinks.reset();
  };
  observePageMetricsAnswers((change: GraphAnswersChange) => {
    reset();
    const rev = BigInt(change.rev);
    batch(() => {
      const reseedModified = modified.delta({ rev, rows: change.pageModified ?? [] });
      const reseedCounts = backlinks.delta({ rev, rows: change.pageBacklinkCounts === undefined ? [] : change.pageBacklinkCounts });
      if (users() > 0) {
        if (reseedModified) void modified.seed(loadModified);
        if (reseedCounts) void backlinks.seed(loadCounts);
      }
    });
  });
  async function loadModified() {
    const result = await readOwned(graphOwner(), backend().pageModified());
    if (result.kind !== "current") throw new Error("All Pages graph changed");
    return { rev: BigInt(result.value.rev), rows: new Map(result.value.modified) };
  }
  async function loadCounts() {
    const result = await readOwned(graphOwner(), backend().pageBacklinkCounts());
    if (result.kind !== "current") throw new Error("All Pages graph changed");
    return { rev: BigInt(result.value[2]), rows: backlinkCountsByRow(result.value) };
  }
  createEffect(() => {
    reset();
    if (users() === 0) return;
    void modified.seed(loadModified);
    void backlinks.seed(loadCounts);
  });
  return {
    retain() { setUsers(n => n + 1); onCleanup(() => setUsers(n => n - 1)); },
    modified: modified.read, backlinks: backlinks.read,
  };
});

/** Acquire the shared metrics for this pane's lifetime. A graph switch clears
 * both maps; ownership guards reject late answers from the previous graph. */
export function useAllPagesMetrics() {
  shared.retain();
  return shared;
}
