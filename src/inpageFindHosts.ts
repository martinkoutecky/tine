import { createMemo, createSignal, onCleanup, untrack, type Accessor } from "solid-js";
import { formatForBlock, node, onPageRawChanged } from "./document";
import { parseBody } from "./render/facets";
import { isQuarantined, parserReady } from "./render/parse";
import { isQueryMacroName } from "./editor/queryMacro";
import { hasQueryMacroCandidate, queryMacroRenderExtents, queryMacroRenderRun } from "./render/queryMacroRender";
import type { Block, Format, Inline, ListItem } from "./render/ast";

/** Parser-owned macro membership, including hosts whose outline window has
 * never mounted. O(block bytes), using the shared body parse cache. */
export function countFindHosts(raw: string, format: Format): number {
  if (!raw.includes("{{") || !parserReady()) return 0;
  const ast = parseBody(raw, format);
  if (isQuarantined(ast)) return 0;
  let count = 0;
  let extents: ReturnType<typeof queryMacroRenderExtents> | undefined;
  const inlines = (values: Inline[]) => {
    const accepted = hasQueryMacroCandidate(values)
      ? queryMacroRenderRun(values, raw, format, extents ??= queryMacroRenderExtents(raw)).map(entry => entry.inline) : values;
    for (const value of accepted) {
      if (value.k === "macro" && (value.name.toLowerCase() === "embed" || isQueryMacroName(value.name))) count++;
      else if ("children" in value) inlines(value.children);
      else if (value.k === "link" && value.label) inlines(value.label);
    }
  };
  const item = (value: ListItem) => {
    if (value.name) inlines(value.name);
    blocks(value.content);
    value.items.forEach(item);
  };
  const blocks = (values: Block[]) => {
    for (const value of values) {
      if ("inline" in value) inlines(value.inline);
      else if (value.kind === "quote" || value.kind === "custom") blocks(value.children);
      else if (value.kind === "list") value.items.forEach(item);
      else if (value.kind === "table") {
        value.header?.forEach(inlines);
        value.rows.forEach(row => row.forEach(inlines));
      }
    }
  };
  blocks(ast);
  return count;
}

const HOST = "[data-find-host]";
const UNSEARCHED = '[data-find-answered="false"],[data-find-source-mounted="false"],.ast-deferred,.sheet-cell-defer,.query-pending-groups';
type Scope = { roots: readonly string[]; format: Format; element: HTMLElement | null };

/** Pending in-outline macro hosts, independent of the search query. Inventory
 * costs O(outline + source bytes) on scope/structure changes; raw edits patch
 * one block. Mount updates inspect only changed macro subtrees, with no IPC or
 * query execution. refresh runs in the same batch as Find's match refresh. */
export function createPendingFindHosts(scopes: Accessor<Scope[]>): { count: Accessor<number>; refresh: (records: MutationRecord[]) => void } {
  const [revision, setRevision] = createSignal(0);
  const inventory = createMemo(() => {
    const members = new Map<string, number>();
    const totals = new Map<string, number>();
    const searchedCounts = new Map<string, number>();
    const pages = new Set<string>();
    const mounted = new Map<Element, { searched: boolean; blockers: Set<Element> }>();
    let pending = 0;
    const scopeList = scopes();
    const accepted = (host: Element) => scopeList.some(scope => scope.element?.contains(host))
      && !host.closest("[data-inpage-find-references]")
      && !host.parentElement?.closest(HOST)
      && totals.has(host.getAttribute("data-find-host") ?? "");
    const changeSearched = (host: Element, delta: number) => {
      const id = host.getAttribute("data-find-host")!;
      const total = totals.get(id) ?? 0, previous = searchedCounts.get(id) ?? 0;
      const next = previous + delta;
      pending += Math.max(0, total - next) - Math.max(0, total - previous);
      if (next) searchedCounts.set(id, next); else searchedCounts.delete(id);
    };
    const update = (host: Element, changes?: Set<Element>) => {
      const previous = mounted.get(host);
      if (!host.isConnected || !accepted(host)) {
        if (previous?.searched) changeSearched(host, -1);
        mounted.delete(host);
        return;
      }
      const blockers = previous?.blockers ?? new Set(host.querySelectorAll(UNSEARCHED));
      if (previous && changes) for (const element of changes) {
        if (element !== host && host.contains(element) && element.matches(UNSEARCHED)) blockers.add(element);
        else blockers.delete(element);
      }
      const searched = host.getAttribute("data-find-answered") === "true" && blockers.size === 0;
      mounted.set(host, { searched, blockers });
      changeSearched(host, Number(searched) - Number(previous?.searched ?? false));
    };
    for (const scope of scopeList) {
      const visit = (ids: readonly string[]) => {
        for (const id of ids) {
          const block = node(id);
          if (!block) continue;
          const total = untrack(() => countFindHosts(block.raw, scope.format));
          members.set(id, (members.get(id) ?? 0) + 1);
          pages.add(block.page);
          if (total) totals.set(id, (totals.get(id) ?? 0) + total);
          pending += total;
          visit(block.children);
        }
      };
      visit(scope.roots);
    }
    for (const scope of scopeList) for (const host of scope.element?.querySelectorAll(HOST) ?? []) update(host);
    onCleanup(onPageRawChanged(id => {
      const copies = members.get(id);
      if (!copies) return;
      const total = countFindHosts(node(id)?.raw ?? "", formatForBlock(id)) * copies;
      const searched = searchedCounts.get(id) ?? 0;
      pending += Math.max(0, total - searched) - Math.max(0, (totals.get(id) ?? 0) - searched);
      if (total) totals.set(id, total); else totals.delete(id);
      setRevision(n => n + 1);
    }, pages));
    return { pending: () => pending, update };
  });
  return {
    count: () => { revision(); return inventory().pending(); },
    refresh: records => {
      const current = inventory();
      const dirty = new Map<Element, Set<Element>>();
      const mark = (host: Element, changes: Element[]) => {
        const affected = dirty.get(host) ?? new Set<Element>();
        for (const changed of changes) affected.add(changed);
        dirty.set(host, affected);
      };
      for (const record of records) {
        const target = record.target.nodeType === 1 ? record.target as Element : record.target.parentElement;
        const changes: Element[] = target ? [target] : [];
        for (const changed of [...record.addedNodes, ...record.removedNodes]) {
          if (changed.nodeType !== 1) continue;
          const element = changed as Element;
          for (const descendant of [element, ...element.querySelectorAll("*")]) {
            changes.push(descendant);
            if (descendant.matches(HOST)) mark(descendant, []);
          }
        }
        for (let host = target?.closest(HOST); host; host = host.parentElement?.closest(HOST)) mark(host, changes);
      }
      for (const [host, changes] of dirty) current.update(host, changes);
      if (dirty.size) setRevision(n => n + 1);
    },
  };
}
