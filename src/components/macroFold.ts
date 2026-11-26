import { createComputed, createSignal, onCleanup, useContext } from "solid-js";
import { blockPageReadOnly, blockProperty, blockWritable, formatForBlock, liveDtoBlock, node as docNode, reportQueryFoldOption, setBlockProperty, setCollapsed, storedFold } from "../document";
import { embedBlockTarget, embedMacroArg } from "../embedTarget";
import { resolvedBlockRefSync } from "../resolveBatch";
import { CollapseSurfaceContext, type CollapseSurfaceApi } from "./Block";
import { readOnlyFold } from "./block/readOnlyFold";
import { isBlockEmbedMacro } from "./blockParts";
import { detectMacro } from "./Rendered";

/** This row's fold in the current surface, with its stored fold as the fallback.
 *  Macro bodies fold with their host row. */
export function hostFolded(surface: CollapseSurfaceApi | null, id: string): boolean {
  const stored = storedFold(id);
  return (blockPageReadOnly(id) ? readOnlyFold : surface)?.collapsed(id, stored) ?? stored;
}

/** A whole-block embed root's fold: its host's `collapsed::` override, else the
 *  source's own fold (GH #360). Read by the root row's arrow and the fold commands. */
export function embedRootFold(hostId: string, sourceFold: boolean): boolean {
  const value = blockProperty(hostId, "collapsed")?.toLowerCase();
  return value === "true" ? true : value === "false" ? false : sourceFold;
}

/** Fold a whole-block embed root: the host owns the choice, the source is untouched. */
export function setEmbedRootFold(hostId: string, collapsed: boolean): void {
  setBlockProperty(hostId, "collapsed", String(collapsed));
}

/** GH #1032: the source whose root row stands at whole-block embed host `hostId`
 *  for the fold commands, or null when the host keeps its own answer. It
 *  qualifies when the host has no children of its own and is writable, and its
 *  source is resolved, loaded, writable, not itself such a host, and still the
 *  block the resolved answer names (the mounted row's identity check). Cost:
 *  one macro detection of the host (and of a qualifying source) and one cache
 *  lookup; no IPC. */
export function embedHostSource(hostId: string): string | null {
  const host = docNode(hostId);
  if (!host || host.children.length || !blockWritable(hostId)) return null;
  const macro = detectMacro(host.raw, formatForBlock(hostId));
  const uuid = isBlockEmbedMacro(macro) ? embedBlockTarget(embedMacroArg(macro!.inner)) : null;
  const group = uuid ? resolvedBlockRefSync(uuid) : null;
  const source = group?.blocks[0] ? liveDtoBlock(group.blocks[0], group) : null;
  if (!source || !blockWritable(source)) return null;
  return isBlockEmbedMacro(detectMacro(docNode(source)!.raw, formatForBlock(source))) ? null : source;
}

/** A query's results fold. GH #619 Q8: for a query block this IS the bullet's
 *  fold, one state with two controls (OG query.cljs `custom-query*` reads the
 *  block's `:block/collapsed?`), so the query's arrow reads and writes the
 *  bullet's `collapsed::` (or a reference/query result's local fold). The `:collapsed?`
 *  option stays the default of a block that has no
 *  `collapsed::` of its own; expanding such a block writes `collapsed:: false`.
 *
 *  A read-only host shares the bullet's session-only presentation fold. A
 *  query with no host block (a journal default query) retains its existing
 *  device-local toggle (`remembered`, written back through `remember`). */
export function createQueryFold(remembered: boolean | null, sourceDefault: () => boolean,
  hostId: () => string | undefined, remember: (collapsed: boolean) => void) {
  const surface = useContext(CollapseSurfaceContext);
  // Report the authored default before the first paint, so the bullet agrees.
  createComputed(() => {
    const id = hostId();
    if (!id || !docNode(id) || !sourceDefault()) return;
    onCleanup(reportQueryFoldOption(id));
  });
  const shared = () => { const id = hostId(); return !!id && !!docNode(id); };
  const [override, setOverride] = createSignal(remembered);
  const bulletFolded = () => { const id = hostId(); return !!id && hostFolded(surface, id); };
  const collapsed = () => shared() ? bulletFolded() : bulletFolded() || (override() ?? sourceDefault());
  const toggle = () => {
    const id = hostId();
    if (shared() && id) {
      if (blockPageReadOnly(id)) readOnlyFold.toggle(id, collapsed());
      else if (surface) surface.toggle(id, collapsed());
      else setCollapsed(id, !collapsed());
      return;
    }
    const v = !collapsed();
    setOverride(v);
    remember(v);
  };
  return { collapsed, toggle };
}
