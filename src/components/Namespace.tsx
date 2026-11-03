import { PageIcon } from "./PageIcon";
import { t } from "../i18n";
import { For, Show, createComputed, createMemo, createSignal, type JSX } from "solid-js";
import { ReferenceDisclosure } from "./ReferenceDisclosure";
import { sectionOverride, setSectionOverride } from "../referenceSectionState";
import { pageIdentityKey } from "../pageIdentity";
import { pageIcon } from "../pageIconBatch";
import { openPage, openPageInNewTab } from "../router";
import { openRouteInOtherPane } from "../panes";
import { internalLinkAuxClick, internalLinkDest, internalLinkMouseDown } from "../linkGesture";
import { openPageInSidebar } from "../ui";
import { allPageNames } from "../pages";
import { allPageNameKeys } from "../pageIndex";
import { EmojiText } from "../render/emoji";
import { SidebarTitle } from "./SidebarTitle";
import type { PageKind } from "../types";
import { shouldOpenTextContextMenu } from "../contextMenuPolicy";

// Namespace hierarchy for a page named `a/b/c`: a clickable breadcrumb of the
// ancestor namespaces (shown above the title) and a list of direct child pages
// (shown below the page). Mirrors OG's hierarchy component.

/** Breadcrumb of ancestor namespaces, e.g. for "a/b/c" → a › b (clickable). */
/** The shared modified-click contract (linkGesture.ts) for a namespace segment:
 *  Shift → right sidebar (GH #63), Ctrl/Cmd or middle → background tab
 *  (GH #283), Alt → other pane (GH #438). `stop` keeps a row's own click
 *  handler from also firing. */
export function nsLink(name: string, after?: () => void, stop = false) {
  return {
    "data-ref": pageIdentityKey(name),
    onMouseDown: internalLinkMouseDown,
    onClick: (e: MouseEvent) => {
      if (stop) e.stopPropagation();
      const dest = internalLinkDest(e);
      if (dest === "sidebar") openPageInSidebar(name, "page");
      else if (dest === "background") openPageInNewTab(name, "page");
      else if (dest === "pane") openRouteInOtherPane({ kind: "page", name, pageKind: "page" });
      else { openPage(name, "page"); after?.(); }
    },
    onAuxClick: (e: MouseEvent) => internalLinkAuxClick(e, () => openPageInNewTab(name, "page")),
  };
}

export function NamespaceCrumb(props: { name: string }): JSX.Element {
  const parts = () => props.name.split("/");
  return (
    <Show when={parts().length > 1}>
      <div class="ns-crumb">
        <For each={parts().slice(0, -1)}>
          {(_, i) => {
            const prefix = () => parts().slice(0, i() + 1).join("/");
            return (
              <>
                <span class="ns-crumb-item" {...nsLink(prefix())}>
                  {parts()[i()]}
                </span>
                <span class="ns-crumb-sep">/</span>
              </>
            );
          }}
        </For>
      </div>
    </Show>
  );
}

// --- Sidebar namespace tree -------------------------------------------------

export interface NsNode {
  seg: string;
  full: string;
  children: NsNode[];
}

/** The namespace tree of the inventory's names (CM step 2 batch 10, model rule 1):
 *  every node is a listed name, nested under the longest proper `/`-prefix of it
 *  that the list also holds. The backend lists every namespace parent
 *  (`InventoryEntry.namespace_parent`), so no level is synthesized here; a name
 *  whose prefixes are not listed (only an empty-key prefix, as in `/x`) is a top
 *  node. `seg` is the part of the name below its parent. Names with no `/` and no
 *  listed name below them are not namespaces and have no node. O(names x depth). */
export function buildNamespaceTree(names: readonly string[]): NsNode[] {
  // Every name with `/` is a node; its proper prefixes' keys are the only keys a
  // name without `/` can usefully have (one key call per segment, as before).
  const nodes = new Map<string, NsNode>();
  const prefixKeys = new Map<NsNode, string[]>();
  const wanted = new Set<string>();
  for (const name of names) {
    if (!name.includes("/")) continue;
    const key = pageIdentityKey(name);
    if (nodes.has(key)) continue;
    const node: NsNode = { seg: name, full: name, children: [] };
    nodes.set(key, node);
    const segs = name.split("/");
    const keys: string[] = [];
    for (let depth = 1; depth < segs.length; depth++) {
      const prefixKey = pageIdentityKey(segs.slice(0, depth).join("/"));
      keys.push(prefixKey);
      wanted.add(prefixKey);
    }
    prefixKeys.set(node, keys);
  }
  // A name without `/` joins only as some node's prefix. `allPageNames()`
  // already holds every name's key (`allPageNameKeys`); any other list pays
  // one key call per name.
  const known = allPageNameKeys(names);
  names.forEach((name, at) => {
    if (name.includes("/")) return;
    const key = known?.[at] ?? pageIdentityKey(name);
    if (wanted.has(key) && !nodes.has(key)) nodes.set(key, { seg: name, full: name, children: [] });
  });
  const roots: NsNode[] = [];
  const placed = new Set<NsNode>();
  for (const [node, keys] of prefixKeys) {
    const segs = node.full.split("/");
    let parent: NsNode | undefined;
    let depth = keys.length;
    for (; depth > 0; depth--) {
      const candidate = nodes.get(keys[depth - 1]);
      if (candidate && candidate !== node) { parent = candidate; break; }
    }
    node.seg = parent ? segs.slice(depth).join("/") : node.full;
    if (parent) {
      parent.children.push(node);
      if (!placed.has(parent) && !prefixKeys.has(parent)) {
        placed.add(parent);
        roots.push(parent);
      }
    } else {
      roots.push(node);
    }
  }
  const pending = [roots];
  while (pending.length) {
    const level = pending.pop()!;
    level.sort((a, b) => a.seg.localeCompare(b.seg));
    for (const node of level) if (node.children.length) pending.push(node.children);
  }
  return roots;
}

const treeOfNames = new WeakMap<readonly string[], NsNode[]>();

/** `buildNamespaceTree` for a name list, built once per list object. The shared
 *  inventory (`allPageNames()`) hands out the same array until the graph's page set
 *  changes, so the sidebar tree and the full Namespaces view share one build instead
 *  of each paying O(names x depth) identity-key calls. Callers must not mutate it. */
export function namespaceTreeFor(names: readonly string[]): NsNode[] {
  let tree = treeOfNames.get(names);
  if (!tree) {
    tree = buildNamespaceTree(names);
    treeOfNames.set(names, tree);
  }
  return tree;
}

function NsNodeView(props: {
  node: NsNode;
  depth: number;
  onPageContextMenu?: (e: MouseEvent, name: string, kind: PageKind) => void;
  onActiveNavigationComplete?: () => void;
}): JSX.Element {
  const [open, setOpen] = createSignal(props.depth < 1);
  const has = () => props.node.children.length > 0;
  return (
    <div class="ns-node">
      <div class="ns-node-row" style={{ "padding-left": `${props.depth * 12}px` }}>
        <Show when={has()} fallback={<span class="ns-node-spacer" />}>
          <ReferenceDisclosure class="ns-node-toggle" collapsed={!open()} onClick={() => setOpen(!open())} />
        </Show>
        <span
          class="ns-node-label"
          {...nsLink(props.node.full, props.onActiveNavigationComplete)}
          onContextMenu={(e) => {
            if (!shouldOpenTextContextMenu(e.target)) return;
            props.onPageContextMenu?.(e, props.node.full, "page");
          }}
        >
          <SidebarTitle text={props.node.seg} fullTitle={props.node.full} />
        </span>
      </div>
      <Show when={has() && open()}>
        <For each={props.node.children}>
          {(c) => (
            <NsNodeView
              node={c}
              depth={props.depth + 1}
              onPageContextMenu={props.onPageContextMenu}
              onActiveNavigationComplete={props.onActiveNavigationComplete}
            />
          )}
        </For>
      </Show>
    </div>
  );
}

/** A collapsible tree of all namespaces in the graph, for the left sidebar. */
export function NamespaceTree(props: {
  onPageContextMenu?: (e: MouseEvent, name: string, kind: PageKind) => void;
  onActiveNavigationComplete?: () => void;
} = {}): JSX.Element {
  // Pure CPU derivation off the shared page-name inventory (src/pageIndex.ts,
  // one page_inventory for every consumer), not its own whole-graph fetch.
  const tree = createMemo(() => namespaceTreeFor(allPageNames()));
  return (
    <Show when={tree().length > 0}>
      <div class="ns-tree">
        <For each={tree()}>
          {(n) => <NsNodeView node={n} depth={0} onPageContextMenu={props.onPageContextMenu} onActiveNavigationComplete={props.onActiveNavigationComplete} />}
        </For>
      </div>
    </Show>
  );
}

// --- {{namespace X}} macro --------------------------------------------------

/** Preorder rows without recursion, even for deeply imported names. Cost O(nodes). */
export function namespaceRows(nodes: NsNode[]): { node: NsNode; depth: number }[] {
  const rows: { node: NsNode; depth: number }[] = [];
  const pending = nodes.map(node => ({ node, depth: 0 })).reverse();
  while (pending.length) {
    const row = pending.pop()!;
    rows.push(row);
    for (let i = row.node.children.length - 1; i >= 0; i--) {
      pending.push({ node: row.node.children[i], depth: row.depth + 1 });
    }
  }
  return rows;
}

function NsMacroNode(props: { node: NsNode; depth: number }): JSX.Element {
  return (
    <div class="ns-macro-node">
      <div class="ns-macro-row" style={{ "padding-left": `${props.depth * 18}px` }}>
        <span class="ns-bullet" />
        <Show when={pageIcon(props.node.full)}>
          <span class="page-icon">
            <PageIcon value={pageIcon(props.node.full)} name={props.node.full} />
          </span>
        </Show>
        <a class="page-ref" {...nsLink(props.node.full, undefined, true)}>
          <EmojiText text={props.node.seg} />
        </a>
      </div>
    </div>
  );
}

/** The node of namespace `root` in the tree of `names` (the inventory's names;
 *  see `buildNamespaceTree`), or null when the list does not hold `root` or
 *  nothing lies below it. Cost O(names x depth). */
export function namespaceNode(names: readonly string[], root: string): NsNode | null {
  const key = pageIdentityKey(root);
  const pending = [...buildNamespaceTree(names)];
  while (pending.length) {
    const node = pending.pop()!;
    if (pageIdentityKey(node.full) === key) return node.children.length ? node : null;
    pending.push(...node.children);
  }
  return null;
}

/** `{{namespace X}}` — the full nested descendant tree of namespace `X`, each
 *  page showing its `icon::` (like OG's namespace macro). */
export function NamespaceMacro(props: { root: string }): JSX.Element {
  // The descendant tree is a pure CPU derivation off the shared page-name
  // inventory (src/pages.ts). Icons use the shared reactive batched lookup,
  // including unsaved loaded-page edits and publications for unloaded pages.
  const treeData = createMemo(() => {
    const key = pageIdentityKey(props.root);
    const prefix = `${key}/`;
    const names = allPageNames().filter((n) => {
      const nameKey = pageIdentityKey(n);
      return nameKey === key || nameKey.startsWith(prefix);
    });
    const node = namespaceNode(names, props.root);
    return { tree: node ? [node] : [] };
  });
  const iconOf = pageIcon;
  return (
    <Show
      when={treeData().tree.length > 0}
      fallback={<span class="macro">{`{{namespace ${props.root}}}`}</span>}
    >
      {/* OG renders a bold "Namespace " label + the root page link as a header
         (components/block.cljs `namespace-hierarchy`), then the descendant tree
         below it — so render the root's children, not the root, as the tree. */}
      <For each={treeData().tree}>
        {(root) => (
          <div class="ns-macro">
            <div class="ns-macro-head">
              <span class="ns-macro-label">{t("namespace-macro-label")}</span>
              <Show when={iconOf(root.full)}>
                <span class="page-icon">
                  <PageIcon value={iconOf(root.full)!} name={root.full} />
                </span>
              </Show>
              <a class="page-ref" {...nsLink(root.full, undefined, true)}>
                <EmojiText text={root.full} />
              </a>
            </div>
            <div class="ns-macro-tree">
              <For each={namespaceRows(root.children)}>
                {(row) => <NsMacroNode node={row.node} depth={row.depth} />}
              </For>
            </div>
          </div>
        )}
      </For>
    </Show>
  );
}

/** Breadcrumb rows for OG's "Hierarchy" section of page `name`: one row per
 *  listed name below it (the inventory lists every namespace parent, so an
 *  intermediate level with no file of its own has its row, as in OG, where
 *  `a/b/c` makes page entities for `a/b` and `a` and `get-namespace-pages`
 *  returns each), as that name's segments, in key order. Nothing is synthesized
 *  from a descendant's spelling. A namespaced leaf with nothing below it gets
 *  one row: its own parent path, the segments of its own name (as
 *  `NamespaceCrumb` shows them; declared: a split of the page's own name, not a
 *  page set). O(names), and no key call per name for an `allPageNames()` list. */
export function namespaceHierarchyRows(allNames: readonly string[], name: string): string[][] {
  const prefix = `${pageIdentityKey(name)}/`;
  const known = allPageNameKeys(allNames);
  const below: [string, string[]][] = [];
  allNames.forEach((n, at) => {
    if (!n.includes("/")) return;
    const key = known?.[at] ?? pageIdentityKey(n);
    if (key.startsWith(prefix)) below.push([key, n.split("/")]);
  });
  if (below.length) {
    return below.sort((a, b) => a[0].localeCompare(b[0])).map(([, segs]) => segs);
  }
  if (name.includes("/")) return [name.split("/").slice(0, -1)];
  return [];
}

/** OG's automatic "Hierarchy" section (components/hierarchy.cljs `structures`):
 *  rendered below any non-journal page that participates in a namespace, as a
 *  bulleted list of breadcrumb paths — one per namespace level (see
 *  `namespaceHierarchyRows`). Each segment links to its cumulative path. */
export function NamespaceHierarchy(props: { name: string }): JSX.Element {
  // Was a createResource keyed on the page NAME → it re-pulled the WHOLE page list
  // over IPC on every navigation (this renders below every non-journal page). Now a
  // pure CPU scan over the shared page-name inventory (src/pages.ts): recomputes
  // on nav, but with zero IPC and no whole-graph deserialize per nav.
  const rows = createMemo(() => namespaceHierarchyRows(allPageNames(), props.name));
  const [collapsed, setCollapsed] = createSignal(false);
  createComputed(() => setCollapsed(sectionOverride("hierarchy", props.name) ?? false));
  const toggle = () => {
    const next = !collapsed();
    setSectionOverride("hierarchy", props.name, next);
    setCollapsed(next);
  };
  return (
    <Show when={rows().length > 0}>
      <div class="page-hierarchy">
        <button type="button" class="references-header hierarchy-toggle" aria-expanded={!collapsed()} onClick={toggle}>
          <ReferenceDisclosure collapsed={collapsed()} />
          {t("namespace-hierarchy")}
        </button>
        <Show when={!collapsed()}>
        <ul class="ns-hierarchy">
          <For each={rows()}>
            {(segs) => (
              <li class="ns-hier-row">
                <span class="ns-bullet" />
                <span class="ns-hier-path">
                  <For each={segs}>
                    {(seg, i) => {
                      const full = () => segs.slice(0, i() + 1).join("/");
                      return (
                        <>
                          <Show when={i() > 0}>
                            <span class="ns-hier-sep">/</span>
                          </Show>
                          <a
                            class="page-ref"
                            {...nsLink(full(), undefined, true)}
                          >
                            <span class="bracket">[[</span>
                            {seg}
                            <span class="bracket">]]</span>
                          </a>
                        </>
                      );
                    }}
                  </For>
                </span>
              </li>
            )}
          </For>
        </ul>
        </Show>
      </div>
    </Show>
  );
}
