/** Mounted query dependencies use the existing publication/key layer. The
 * host owns classification; this module only schedules affected readers. */
import { createEffect, createMemo, createSignal, onCleanup, untrack, type Accessor } from "solid-js";
import { backend } from "./backend";
import { captureBinding } from "./binding";
import { bindingOwner, readOwned } from "./owned";
import { graphEpoch, dataRev } from "./graphSession";
import { changeSummary } from "./changeSummary";
import { changedAny, refreshEverything } from "./changeKeys";
import { resultsSettled } from "./queryResultsEpoch";
import { onEachWindow } from "./windowRealm";
import type { Query, ExecutionContext } from "./editor/queryIr";

export type QuerySubscription = { kind: "query"; query: Query; context: ExecutionContext }
  | { kind: "source"; source: string; dialect: import("./editor/queryIr").QueryTextDialect }
  | { kind: "registry" } | { kind: "full" };
export interface QueryRegistration { mode: "narrow" | "quiet" | "full"; rev: string }
export const QUERY_QUIET_MS = 1500;
const [inputRevision, setInputRevision] = createSignal(0);
let lastInput = -Infinity;
/** The document dirty funnel calls this on each accepted edit, before the
 * host debounce. Publication arrival is deliberately not the typing clock. */
export function noteQueryInput(): void { lastInput = Date.now(); setInputRevision(n => n + 1); }
// One app-lifetime observer per registered document, including composition
// input that has not reached the document dirty funnel yet. Workspace-window
// disposal removes its listeners; programmatic edits still use that funnel.
onEachWindow(win => {
  const doc = win.document;
  if (!doc?.addEventListener) return;
  doc.addEventListener("input", noteQueryInput, true);
  doc.addEventListener("compositionupdate", noteQueryInput, true);
  return () => {
    doc.removeEventListener("input", noteQueryInput, true);
    doc.removeEventListener("compositionupdate", noteQueryInput, true);
  };
});

export function createQuerySubscription(question: Accessor<QuerySubscription | undefined>, day: Accessor<number | undefined> = () => undefined) {
  const descriptor = createMemo(() => JSON.stringify([graphEpoch(), day(), question() ?? null]));
  const subscription = createMemo(() => {
    descriptor();
    const request = untrack(question);
    const host = backend();
    const register = host.registerQuerySubscription?.bind(host);
    const generation = captureBinding().backendGeneration;
    const owner = bindingOwner();
    let active = true;
    const id = crypto.randomUUID();
    const [mode, setMode] = createSignal<"narrow" | "quiet" | "full">("full");
    let baseline = "0";
    const renew = async () => {
      if (!active || !owner() || !request || !register) return;
      const result = await readOwned(owner, register(id, request, generation));
      if (result.kind === "current" && active) { baseline = result.value.rev; setMode(result.value.mode); }
    };
    let preparation = renew();
    let initial = true;
    const prepare = () => {
      if (initial) initial = false;
      // Renew global plan snapshots only at an actual read, after quiet and
      // publication gates. Serial order also makes cleanup retire late renewals.
      else preparation = preparation.then(renew, renew);
      return preparation.then(() => active && owner());
    };
    onCleanup(() => {
      active = false;
      if (!request || !register) return;
      const release = async () => {
        if (owner()) await readOwned(owner, register(id, null, generation));
      };
      void preparation.then(release, release).catch(() => { if (owner()) console.warn("query subscription cleanup failed"); });
    });
    return { id, prepare, mode, registered: !!register, baseline: () => baseline };
  });
  const raw = createMemo(() => `${day() ?? ""}:${subscription().registered ? changedAny([`query:${subscription().id}`]) : `${graphEpoch()}:${dataRev()}`}`);
  const [quietRevision, setQuietRevision] = createSignal(untrack(raw));
  createEffect(() => {
    const current = raw();
    const quiet = subscription().mode() === "quiet" && !refreshEverything();
    inputRevision();
    const settled = resultsSettled();
    if (!quiet) { setQuietRevision(current); return; }
    if (current === untrack(quietRevision) || !settled) return;
    const delay = Math.max(0, QUERY_QUIET_MS - (Date.now() - lastInput));
    const timer = setTimeout(() => {
      // Both are rechecked at execution: no stale-publication run can slip
      // between a timer and a newly accepted edit or publication wait.
      if (resultsSettled() && Date.now() - lastInput >= QUERY_QUIET_MS) setQuietRevision(current);
    }, delay);
    onCleanup(() => clearTimeout(timer));
  });
  const revision = createMemo(() => subscription().mode() === "quiet" && !refreshEverything() ? quietRevision() : raw());
  return { revision, ready: () => subscription().prepare(),
    // Cache identity is sampled only when a read starts. It never schedules a
    // read. Registration's host revision closes the late-mount/delayed-event
    // race; the summary catches subsequent publications of this reader.
    publication: () => untrack(() => {
      const baseline = subscription().baseline();
      const latest = changeSummary()?.rev ?? "0";
      return `${BigInt(baseline) > BigInt(latest) ? baseline : latest}:${dataRev()}`;
    }) };
}
