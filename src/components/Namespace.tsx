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
import { allPageNameKeys, entityOpening, isNamespaceName, isNamespacePage, linkTarget, namespaceRow, namespaceTarget } from "../pageIndex";
import { isPublishedExport } from "../publishedBackend";
import { EmojiText } from "../render/emoji";
import { PageReferenceLabel } from "../render/PageReferenceLabel";
import { SidebarTitle } from "./SidebarTitle";
import type { PageInventoryEntry, PageKind } from "../types";
import { shouldOpenTextContextMenu } from "../contextMenuPolicy";

// Namespace hierarchy for a page named `a/b/c`: a clickable breadcrumb of the
// ancestor namespaces (shown above the title) and a list of direct child pages
// (shown below the page). Mirrors OG's hierarchy component.

/** Breadcrumb of ancestor namespaces, e.g. for "a/b/c" → a › b (clickable). */
/** The shared modified-click contract (linkGesture.ts) for a namespace segment:
 *  Shift → right sidebar (GH #63), Ctrl/Cmd or middle → background tab
 *  (GH #283), Alt → other pane (GH #438). `stop` keeps a row's own click
 *  handler from also firing. */
export function nsLink(name: string, after?: () => void, stop = false, kind: PageKind = "page") {
  const opening = () => entityOpening(name, kind);
  return {
    "data-ref": pageIdentityKey(name),
    onMouseDown: internalLinkMouseDown,
    onClick: (e: MouseEvent) => {
      if (stop) e.stopPropagation();
      const dest = internalLinkDest(e);
      const target = opening();
      if (dest === "sidebar") openPageInSidebar(target.name, target.kind);
      else if (dest === "background") openPageInNewTab(target.name, target.kind);
      else if (dest === "pane") openRouteInOtherPane({ kind: "page", name: target.name, pageKind: target.kind });
      else { openPage(target.name, target.kind); after?.(); }
    },
    onAuxClick: (e: MouseEvent) => internalLinkAuxClick(e, () => { const target = opening(); openPageInNewTab(target.name, target.kind); }),
  };
}

export function NamespaceCrumb(props: { name: string }): JSX.Element {
  // `./x` or a URL is no namespace name (the store's answer): no crumb.
  const parts = () => (isNamespaceName(props.name) ? props.name.split("/") : [props.name]);
  return (
    <Show when={parts().length > 1}>
      <div class="ns-crumb">
        <For each={parts().slice(0, -1)}>
          {(_, i) => {
            const prefix = () => parts().slice(0, i() + 1).join("/");
            const target = () => isPublishedExport() ? { name: prefix(), kind: "page" as const } : linkTarget(prefix());
            return (
              <>
                <span class="ns-crumb-item" {...nsLink(target().name, undefined, false, target().kind)}>
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
  kind?: PageKind;
}

/** The typed page a tree row's actions target: the row's inventory kind, an
 *  absent day by its safe opening spelling (`entityOpening`). Every
 *  namespace view's row menu, favorite and opening read it. */
export function nsNodeTarget(node: NsNode): { name: string; kind: PageKind } {
  return entityOpening(node.full, node.kind ?? "page");
}

/** The namespace tree of the inventory's names (CM step 2 batch 10, model rule 1):
 *  every node is a listed page, nested under its row's parent candidate
 *  (`InventoryEntry::parent`) exactly when that typed page is a node. Rust
 *  excludes alias-only names from this universe.
 *  The backend lists every namespace parent
 *  (`InventoryEntry.namespace_parent`), so no level is synthesized here; a name
 *  without a parent is a top node, shown when something lies below it or
 *  when its row is a day the store marks (`InventoryEntry::namespace_day`). `seg` is
 *  the part of the name below its parent. Names that are not
 *  namespace names (no `/`, or the store's `not_namespace`: `./x`, a URL; model
 *  round 10, S15) and have no listed name below them have no node.
 *  O(names x length) when the list holds each name's immediate prefix, as the
 *  inventory does. */
export function buildNamespaceTree(names: readonly string[]): NsNode[] {
  // Published answers v1 is frozen (ANSWERS-V2): keep its former namespace
  // interpretation until the publication format can carry the live answers.
  const legacy = isPublishedExport();
  const identity = (name: string) => legacy ? { name, kind: "page" as const } : namespaceTarget(name)!;
  const typedKey = (name: string, kind: PageKind = "page") => `${kind}:${pageIdentityKey(name)}`;
  // Every namespace name is a node. A name without `/` can only be a first
  // segment, so only first-segment keys are wanted from plain names.
  const nodes = new Map<string, NsNode>();
  const slashed: [NsNode, string[], PageInventoryEntry | undefined][] = [];
  const wanted = new Set<string>();
  for (const name of names) {
    if (!name.includes("/")) continue;
    const rawKey = pageIdentityKey(name);
    const target = identity(name);
    const key = typedKey(target.name, target.kind);
    if (nodes.has(key) || !isNamespaceName(name, rawKey) || !isNamespacePage(name, rawKey)) continue;
    const node: NsNode = { seg: name, full: name, children: [], ...(target.kind === "journal" ? { kind: target.kind } : {}) };
    nodes.set(key, node);
    const segs = name.split("/");
    const row = legacy ? undefined : namespaceRow(name);
    slashed.push([node, segs, row]);
    const parent = legacy ? { name: segs[0], kind: "page" as const } : row?.parent;
    if (parent) wanted.add(typedKey(parent.name, parent.kind));
  }
  // `allPageNames()` already holds every name's key (`allPageNameKeys`); any
  // other list pays one key call per plain name.
  const known = allPageNameKeys(names);
  names.forEach((name, at) => {
    if (name.includes("/")) return;
    const rawKey = known?.[at] ?? pageIdentityKey(name);
    if (!wanted.has(`page:${rawKey}`) && !wanted.has(`journal:${rawKey}`)) return;
    const targets = legacy ? [identity(name)] : [namespaceTarget(name, "page"), namespaceTarget(name, "journal")];
    for (const target of targets) {
      if (!target) continue;
      const key = typedKey(target.name, target.kind);
      if (wanted.has(key) && !nodes.has(key) && isNamespacePage(name, rawKey)) nodes.set(key, { seg: name, full: name, children: [], ...(target.kind === "journal" ? { kind: target.kind } : {}) });
    }
  });
  // The live tree attaches each node below its row's parent candidate when
  // that typed page is a node, and keeps a childless top node only when the
  // row is a `namespace_day`; nothing else is checked, so no node gains a
  // more distant ancestor.
  const roots: NsNode[] = [];
  const placed = new Set<NsNode>();
  const kept = new Set<NsNode>();
  for (const [node, segs, row] of slashed) {
    let parent: NsNode | undefined;
    // The label is the part below the parent.
    let depth = segs.length - 1;
    if (legacy) {
      while (depth > 0 && !segs.slice(0, depth).join("/")) depth--;
      const candidate = nodes.get(typedKey(segs.slice(0, depth).join("/")));
      if (depth > 0 && candidate !== node) parent = candidate;
      if (!parent) {
        for (depth--; depth > 0; depth--) {
          const ancestor = nodes.get(typedKey(segs.slice(0, depth).join("/")));
          if (ancestor && ancestor !== node) { parent = ancestor; break; }
        }
      }
    } else if (row?.parent) {
      parent = nodes.get(typedKey(row.parent.name, row.parent.kind));
    }
    // A trailing `/` folds away (`a/b/` lies below `a`): label it `b/`.
    node.seg = parent ? segs.slice(depth).join("/") || segs.slice(depth - 1).join("/") : node.full;
    if (parent) {
      parent.children.push(node);
      if (!placed.has(parent) && !parent.full.includes("/")) {
        placed.add(parent);
        roots.push(parent);
      }
    } else {
      roots.push(node);
      if (row?.namespace_day) kept.add(node);
    }
  }
  const pending = [roots];
  while (pending.length) {
    const level = pending.pop()!;
    level.sort((a, b) => a.seg.localeCompare(b.seg));
    for (const node of level) if (node.children.length) pending.push(node.children);
  }
  return legacy ? roots : roots.filter((node) => node.children.length > 0 || kept.has(node));
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
          {...nsLink(props.node.full, props.onActiveNavigationComplete, false, props.node.kind)}
          onContextMenu={(e) => {
            if (!shouldOpenTextContextMenu(e.target)) return;
            const target = nsNodeTarget(props.node);
            props.onPageContextMenu?.(e, target.name, target.kind);
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
        <a class="page-ref" {...nsLink(props.node.full, undefined, true, props.node.kind)}>
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
  const target = isPublishedExport() ? { name: root, kind: "page" } : namespaceTarget(root);
  const key = pageIdentityKey(target?.name ?? root);
  const kind = target?.kind ?? "page";
  const pending = [...namespaceTreeFor(names)];
  while (pending.length) {
    const node = pending.pop()!;
    if (pageIdentityKey(node.full) === key && (node.kind ?? "page") === kind) return node.children.length ? node : null;
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
    const node = namespaceNode(allPageNames(), props.root);
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
              <a class="page-ref" {...nsLink(root.full, undefined, true, root.kind)}>
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
 *  page set). Only namespace names count (`isNamespaceName`, the store's
 *  answer): `./x` lies below no page `.` and has no row of its own, as in
 *  Logseq (`hierarchy.cljs` asks `namespace-page?`). O(names), and no key call
 *  per name for an `allPageNames()` list. */
export function namespaceHierarchyRows(allNames: readonly string[], name: string): string[][] {
  if (isPublishedExport()) {
    const prefix = `${pageIdentityKey(name)}/`;
    const below = allNames.filter((n) => pageIdentityKey(n).startsWith(prefix) && isNamespaceName(n));
    if (below.length) return below.sort((a, b) => pageIdentityKey(a).localeCompare(pageIdentityKey(b))).map((n) => n.split("/"));
    return isNamespaceName(name) ? [name.split("/").slice(0, -1)] : [];
  }
  if (!isNamespacePage(name)) return [];
  const target = namespaceTarget(name);
  const key = pageIdentityKey(target?.name ?? name);
  const pending = namespaceTreeFor(allNames).map((node) => ({ node, parent: null as NsNode | null }));
  while (pending.length) {
    const { node, parent } = pending.pop()!;
    if (pageIdentityKey(node.full) === key && (node.kind ?? "page") === target?.kind) {
      const below = namespaceRows(node.children).map((row) => row.node.full);
      if (below.length) return below.sort((a, b) => pageIdentityKey(a).localeCompare(pageIdentityKey(b))).map((n) => n.split("/"));
      return parent ? [parent.full.split("/")] : [];
    }
    for (const child of node.children) pending.push({ node: child, parent: node });
  }
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
                      // A prefix opens what its link opens; the last segment is
                      // the row's own page (the page's tree parent, else itself).
                      const target = () => {
                        if (isPublishedExport()) return { name: full(), kind: "page" as const };
                        if (i() < segs.length - 1) return linkTarget(full());
                        const parent = namespaceRow(props.name)?.parent;
                        return parent?.name === full() ? parent : namespaceTarget(full())!;
                      };
                      return (
                        <>
                          <Show when={i() > 0}>
                            <span class="ns-hier-sep">/</span>
                          </Show>
                          <a
                            class="page-ref"
                            {...nsLink(target().name, undefined, true, target().kind)}
                          >
                            <PageReferenceLabel>{seg}</PageReferenceLabel>
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
