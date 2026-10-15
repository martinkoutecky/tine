import { createMemo, createResource, onCleanup, untrack, type Accessor, type Resource } from "solid-js";
import { editingId } from "./editorController";
import { dataRev, graphEpoch } from "./graphSession";
import { node as docNode, isConflicted, isDirty, isSaving, pendingDataRevision } from "./document";
import { graphOwner, readOwned } from "./owned";
import { readOr, readLatestOr } from "./resourceRead";
import { bindingIdentity } from "./binding";
import type { BlockDto, RefGroup } from "./types";
import { adoptResults, captureResultsTicket, resultsEpoch, resultsSettled } from "./queryResultsEpoch";
import { resultDrafts } from "./queryResultDrafts";

export function blocksContainEdit(blocks: BlockDto[], id: string): boolean {
  return blocks.some((block) => block.id === id || blocksContainEdit(block.children, id));
}

export function groupsContainEdit(groups: RefGroup[], id: string): boolean {
  return groups.some((group) => blocksContainEdit(group.blocks, id));
}

/** One read lifecycle for derived result surfaces. Ordinary autosave is untouched.
 * Intentional difference: OG re-runs custom queries mid-edit (react.cljs:335).
 * Tine keeps the rendered answer/position until editing ends and the final save
 * revision settles, then coalesces suppressed invalidations into one request.
 * Identity changes always retire retention; an in-flight read cannot replace an
 * answer underneath its editor or land in a different graph/route (I-20). */
export function createMembershipResource<S, T>(
  source: Accessor<S | undefined | null | false>,
  identity: Accessor<string>,
  load: (source: S) => Promise<T>,
  contains: (value: T, id: string) => boolean,
  invalidate: Accessor<boolean> = () => true,
  guardResults = false,
) {
  let snapshot: T | undefined;
  let scope = "";
  let retained: { id: string; page?: string; epoch: number } | undefined;
  let snapshotEpoch = -1;
  let release = 0;
  let alive = true;
  let resolvedScope = "";
  let answered: { input: S | undefined | null | false; scope: string; revision: number; epoch: number; release: number; guarded: boolean } | undefined;
  onCleanup(() => { alive = false; });
  const inputs = createMemo(source);
  const currentScope = () => `${bindingIdentity()}\0${graphEpoch()}\0${identity()}`;
  const request = createMemo(() => {
    const nextScope = currentScope();
    const input = inputs();
    const revision = invalidate() ? dataRev() : 0;
    // A suppressed (collapsed) read is display-only: never stamped, so never acted on.
    const guarded = guardResults && invalidate();
    const epoch = guarded ? resultsEpoch() : 0;
    const settled = !guarded || resultsSettled();
    const id = editingId();
    if (scope !== nextScope) {
      scope = nextScope;
      snapshot = undefined;
      retained = undefined;
    }
    const draft = snapshot === undefined ? undefined : resultDrafts().find(draft => untrack(() => contains(snapshot!, draft.id)));
    if (draft) return { input, scope, revision, epoch, settled, guarded, release, held: true };
    if (retained && guarded && retained.epoch !== epoch) { retained = undefined; release++; }
    if (id && snapshot !== undefined && (!guarded || snapshotEpoch === epoch) && untrack(() => contains(snapshot!, id))) {
      retained = { id, page: untrack(() => docNode(id)?.page), epoch };
    }
    if (retained) {
      const page = retained.page;
      const unsettled = page && (isDirty(page) || isSaving(page) || isConflicted(page) || pendingDataRevision());
      if (id === retained.id || unsettled) return { input, scope, revision, epoch, settled, guarded, release, held: true };
      retained = undefined;
      release++;
    }
    return { input, scope, revision, epoch, settled, guarded, release, held: false };
  });
  // Holding never starts another scan. Keep a stable source token during the
  // session, even across many autosave revisions; release starts exactly once.
  const runnable = createMemo((previous: ReturnType<typeof request> | undefined) => {
    const next = request();
    if (next.held && previous?.scope === next.scope) return previous;
    return next;
  }, undefined, { equals: (a, b) => a?.scope === b?.scope && a?.input === b?.input
    && a?.revision === b?.revision && a?.release === b?.release && a?.epoch === b?.epoch && a?.settled === b?.settled && a?.guarded === b?.guarded });
  const [resource, actions] = createResource(
    () => runnable()?.input && runnable()?.settled ? runnable() : undefined,
    async (started) => {
      // A permission gate (e.g. clean host Open) does not change the question or
      // its answer. Reuse this resource's current snapshot, without an IPC read.
      if (answered && snapshot !== undefined && answered.scope === started.scope
        && answered.input === started.input && answered.revision === started.revision
        && answered.epoch === started.epoch && answered.release === started.release
        && answered.guarded === started.guarded) return snapshot;
      const owner = graphOwner(() => alive && request().scope === started.scope
        && !request().held && runnable() === started);
      if (request().held) return snapshot;
      const ticket = captureResultsTicket();
      const result = await readOwned(owner, load(started.input as S));
      if (result.kind === "stale") return request().scope === started.scope && request().held ? snapshot : undefined;
      if (request().held) return snapshot;
      if (runnable() !== started) return undefined;
      if (started.guarded && adoptResults(result.value, ticket) === undefined) return undefined;
      snapshot = result.value;
      answered = started;
      snapshotEpoch = ticket.epoch;
      resolvedScope = started.scope;
      return result.value;
    },
  );
  const visible = (() => {
    const value = readOr(resource, undefined, "result membership");
    return resolvedScope === currentScope() ? value : undefined;
  }) as Resource<T | undefined>;
  Object.defineProperties(visible, {
    latest: { get: () => {
      const value = readLatestOr(resource, undefined, "result membership");
      return resolvedScope === currentScope() ? value : undefined;
    } },
    loading: { get: () => resource.loading },
    error: { get: () => resource.error },
    state: { get: () => resource.state },
  });
  return [visible, actions] as const;
}
