// Rename sources whose original text a cancelled rename kept (the host's
// `DraftStatus.restorable`, then its `windowReloaded` reply and page mail when
// the list changes): the unsaved-changes recovery panel (GH #540) lists each
// with Restore and Copy, and the host's record of each restore (restoring,
// refused and why, restored). A leaf like `unsavedRecovery.ts`: graph open and
// the page-mail listener write it, the panel, the launch notice and the
// "review kept originals" command read it. The list belongs to the graph
// binding it came from: invalidation clears it and retires the launch notice.
import { bindingCurrent, captureBinding, clearOnBindingInvalidated, graphScopedSignal } from "./binding";
import type { RestorableList, RestorableSource } from "./document";
import { msg } from "./i18n";
import { dismissToast, pushToast } from "./toasts";
import { openUnsavedRecovery } from "./unsavedRecovery";

/** Graph-scoped (I-20): the list reads as empty once its binding retires,
 * and a Restore from it names the host session it came with. */
const scoped = graphScopedSignal<readonly RestorableSource[]>("binding");
const sources = (): readonly RestorableSource[] => scoped[0]() ?? [];
const setSources = scoped[1];
/** The generation of the list installed for this binding (-1: none yet). */
let generation = -1;
/** The launch notice naming this graph's kept sources, while it shows. */
let notice: number | null = null;

/** The sources the panel lists, in the host's order (finished restores last). */
export const restorableSources = (): readonly RestorableSource[] => sources();

/** The sources a restore could still take: the list without the finished ones.
 * Non-empty exactly while the standing entry (the command) is offered. */
export const keptSources = (): readonly RestorableSource[] =>
  sources().filter((source) => source.state?.kind !== "restored");

/** The host's list at `list.generation`. `known`: the graph-open or session
 * reload reply's list, which starts the session's view (installed whatever its
 * generation) and never opens the panel. A list from page mail is installed
 * only when newer than the installed one: an older or equal one is a replay
 * (stashed page mail) and changes nothing. It opens the panel when it names a
 * source not listed before (a Use Disk just cancelled its rename); one that
 * left only disappears. */
export function setRestorableSources(list: RestorableList, known: boolean): void {
  if (!known && list.generation <= generation) return;
  generation = list.generation;
  const before = new Set(sources().map((source) => source.key));
  setSources(list.sources);
  const fresh = list.sources.some((source) => source.state?.kind !== "restored" && !before.has(source.key));
  if (!known && fresh) openUnsavedRecovery();
}

/** The graph-open notice naming the kept sources, with Review: bound to the
 * graph it names. A graph switch retires it, and a Review that outlives its
 * graph opens nothing. */
export function noticeKeptSources(): void {
  const kept = keptSources();
  if (!kept.length) return;
  const binding = captureBinding();
  const id: number = pushToast(msg("graph-kept-original-text-of-cancelled-renames", { count: kept.length, pages: kept.map((source) => source.path).join(", ") }),
    "info", { sticky: true, action: { label: msg("graph-review-kept-pages"), run: () => {
      dismissToast(id);
      if (notice === id) notice = null;
      if (bindingCurrent(binding)) openUnsavedRecovery();
    } } });
  notice = id;
}

clearOnBindingInvalidated(() => {
  generation = -1;
  setSources(null);
  if (notice !== null) dismissToast(notice);
  notice = null;
});
