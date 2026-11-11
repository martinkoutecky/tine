import { createEffect, createMemo, createSignal, untrack, type Accessor, type JSX } from "solid-js";
import { Rich } from "../i18n/Rich";
import { t, msg } from "../i18n";
import { changed, pageChangeKey } from "../changeKeys";
import { graphOwner, readOwned, type Owner } from "../owned";
import { backend } from "../backend";
import type { Route } from "../router";
import type { PageRead, PageReadSource } from "../types";
import { recordDiagnostic } from "../debug";

/** Compare the exact selection once; successful answers allocate no mismatch record. */
export function pageOpenMismatch(r: Extract<Route, { kind: "page" }>, dto: PageRead | null) {
  const pathMismatch = dto?.id !== r.path;
  const nameMismatch = dto?.name !== r.name;
  const kindMismatch = dto?.kind !== r.pageKind;
  return pathMismatch || nameMismatch || kindMismatch ? { pathMismatch, nameMismatch, kindMismatch } : null;
}

/** Whether a route opens a read under the read's identity instead of refusing it:
 * the same file in another case spelling, or the same file and name with the
 * store's kind (GH #863). The route loader and the recovery probe both ask this. */
export function routeAdoptsRead(r: Extract<Route, { kind: "page" }>, dto: PageRead | null): boolean {
  return !!(r.path && dto?.id && dto.id.toLowerCase() === r.path.toLowerCase()
    && (dto.id !== r.path || (dto.name === r.name && dto.kind !== r.pageKind)));
}

/** Record the comparisons that refused this read, without its identity operands. */
export function recordPageOpenRefusal(stage: "route" | "probe", mismatch: NonNullable<ReturnType<typeof pageOpenMismatch>>, source: PageReadSource | null): void {
  void recordDiagnostic("page_open_refused", { pageOpen: { stage, readSource: source, ...mismatch } });
}

/** The refusal a routed page shows in place of its content when it could not be opened. */
export function PageLoadError(props: { error: string }): JSX.Element {
  return (
    <div class="page">
      <div class="page-load-error">
        <Rich message={msg("page-open-failed", { error: props.error })} />
        <div class="page-load-error-hint">{t("page-ui-tine-did-not-modify-the-file-try-reopening-or")}</div>
      </div>
    </div>
  );
}

/** Why a path-pinned route was refused: its exact file was absent, could not be read, or another page answered. */
export type PinnedPathRefusal = "missing" | "unreadable" | "mismatch";

/**
 * Recovery for a path-pinned page route refused because its exact file was
 * unavailable (GH #863): a sync delivery can briefly remove a file whose title
 * still shows in the feed. The route keeps that exact target and never
 * substitutes a same-name sibling; it retries the full load (with all its
 * identity and admission checks) when the page's publication changes, and,
 * while the file is missing, probes the exact path without touching the
 * refusal UI, from each completed probe after 3 s doubling to 60 s. A file
 * that exists but cannot be read keeps its read error shown and recovers the
 * same way: the first probe that reads it runs the full load again.
 *
 * `begin` starts a load attempt and returns its publication snapshot; `refuse`
 * records a refused attempt and returns the cancel for its probe, if any. The
 * caller owns cancellation (route/graph change, surface retirement, success);
 * `live` is the attempt's route predicate, bound to the current graph here.
 * Cost: one memo per mounted route; at most one exact-path read in flight.
 */
export function createPinnedPathRecovery(route: Accessor<Route>, retry: () => void) {
  const publication = createMemo(() => {
    const r = route();
    return r.kind === "page" && r.path ? changed(pageChangeKey(r.name)) : null;
  });
  const [refusedAt, setRefusedAt] = createSignal<string | null>(null);
  createEffect(() => {
    const at = refusedAt();
    if (at !== null && publication() !== at) retry();
  });
  return {
    begin(): string | null {
      setRefusedAt(null);
      return untrack(publication);
    },
    refuse(at: string | null, refusal: PinnedPathRefusal, r: Route, live: Owner): (() => void) | undefined {
      setRefusedAt(at);
      if (refusal === "mismatch" || r.kind !== "page" || !r.path || !live()) return undefined;
      const owner = graphOwner(live);
      const path = r.path;
      let delay = 3000;
      // A probe records a flight event only when its facts change: the probe
      // repeats for as long as the route stays refused.
      let recorded = "";
      let cancel = () => {};
      const arm = () => {
        const timer = setTimeout(() => { void probe(); }, delay);
        cancel = () => clearTimeout(timer);
      };
      const probe = async () => {
        if (!owner()) return;
        let source: PageReadSource | null = null;
        try {
          const result = await readOwned(owner, backend().getPageByPath(path, value => { source = value; }));
          if (result.kind === "stale") return;
          const dto = result.value;
          const mismatch = pageOpenMismatch(r, dto);
          if (refusal === "unreadable" || !mismatch || routeAdoptsRead(r, dto)) {
            retry();
            return;
          }
          const facts = JSON.stringify([source, mismatch.pathMismatch, mismatch.nameMismatch, mismatch.kindMismatch]);
          if (facts !== recorded) {
            recorded = facts;
            recordPageOpenRefusal("probe", mismatch, source);
          }
        } catch {
          // Keep the original refusal on a probe failure, with no toast or
          // flight event; a publication can still recover the route.
        }
        if (!owner()) return;
        delay = Math.min(delay * 2, 60_000);
        arm();
      };
      arm();
      return () => cancel();
    },
  };
}
