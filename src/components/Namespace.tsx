import { For, Show, createMemo, createResource, createSignal, type JSX } from "solid-js";
import { backend } from "../backend";
import { openPage } from "../router";
import { allPageNames } from "../pages";
import { EmojiText } from "../render/emoji";

// Namespace hierarchy for a page named `a/b/c`: a clickable breadcrumb of the
// ancestor namespaces (shown above the title) and a list of direct child pages
// (shown below the page). Mirrors OG's hierarchy component.

/** Breadcrumb of ancestor namespaces, e.g. for "a/b/c" → a › b (clickable). */
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
                <span class="ns-crumb-item" onClick={() => openPage(prefix(), "page")}>
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

/** Build a nested namespace tree from page names containing `/`. Intermediate
 *  segments become nodes even if they have no file of their own. */
export function buildNamespaceTree(names: string[]): NsNode[] {
  const roots: NsNode[] = [];
  const byFull = new Map<string, NsNode>();
  for (const name of names) {
    if (!name.includes("/")) continue;
    let level = roots;
    let prefix = "";
    for (const seg of name.split("/")) {
      prefix = prefix ? `${prefix}/${seg}` : seg;
      let node = byFull.get(prefix.toLowerCase());
      if (!node) {
        node = { seg, full: prefix, children: [] };
        byFull.set(prefix.toLowerCase(), node);
        level.push(node);
      }
      level = node.children;
    }
  }
  const sortRec = (ns: NsNode[]) => {
    ns.sort((a, b) => a.seg.localeCompare(b.seg));
    ns.forEach((n) => sortRec(n.children));
  };
  sortRec(roots);
  return roots;
}

function NsNodeView(props: { node: NsNode; depth: number }): JSX.Element {
  const [open, setOpen] = createSignal(props.depth < 1);
  const has = () => props.node.children.length > 0;
  return (
    <div class="ns-node">
      <div class="ns-node-row" style={{ "padding-left": `${props.depth * 12}px` }}>
        <Show when={has()} fallback={<span class="ns-node-spacer" />}>
          <span class="ns-node-toggle" onClick={() => setOpen(!open())}>{open() ? "▾" : "▸"}</span>
        </Show>
        <span class="ns-node-label" onClick={() => openPage(props.node.full, "page")}>
          {props.node.seg}
        </span>
      </div>
      <Show when={has() && open()}>
        <For each={props.node.children}>{(c) => <NsNodeView node={c} depth={props.depth + 1} />}</For>
      </Show>
    </div>
  );
}

/** A collapsible tree of all namespaces in the graph, for the left sidebar. */
export function NamespaceTree(): JSX.Element {
  // Pure CPU derivation off the shared, epoch-keyed page list (src/pages.ts) —
  // no longer its own whole-graph listPages() fetch.
  const tree = createMemo(() => buildNamespaceTree(allPageNames()));
  return (
    <Show when={tree().length > 0}>
      <div class="ns-tree">
        <For each={tree()}>{(n) => <NsNodeView node={n} depth={0} />}</For>
      </div>
    </Show>
  );
}

// --- {{namespace X}} macro --------------------------------------------------

function collectFulls(nodes: NsNode[], acc: string[]) {
  for (const n of nodes) {
    acc.push(n.full);
    collectFulls(n.children, acc);
  }
}

function NsMacroNode(props: { node: NsNode; depth: number; icons: Record<string, string> }): JSX.Element {
  return (
    <div class="ns-macro-node">
      <div class="ns-macro-row" style={{ "padding-left": `${props.depth * 18}px` }}>
        <span class="ns-bullet" />
        <Show when={props.icons[props.node.full]}>
          <span class="page-icon">
            <EmojiText text={props.icons[props.node.full]} />
          </span>
        </Show>
        <a class="page-ref" onClick={(e) => { e.stopPropagation(); openPage(props.node.full, "page"); }}>
          <EmojiText text={props.node.seg} />
        </a>
      </div>
      <For each={props.node.children}>
        {(c) => <NsMacroNode node={c} depth={props.depth + 1} icons={props.icons} />}
      </For>
    </div>
  );
}

/** `{{namespace X}}` — the full nested descendant tree of namespace `X`, each
 *  page showing its `icon::` (like OG's namespace macro). */
export function NamespaceMacro(props: { root: string }): JSX.Element {
  // The descendant tree is a pure CPU derivation off the shared, epoch-keyed page
  // list (src/pages.ts). Only the per-page icon lookup stays an IPC, keyed on the
  // resulting `fulls` set (so it refetches when the page set changes, not per nav).
  const treeData = createMemo(() => {
    const prefix = `${props.root}/`.toLowerCase();
    const names = allPageNames().filter((n) => n.toLowerCase().startsWith(prefix));
    const tree = buildNamespaceTree(names);
    const fulls: string[] = [];
    collectFulls(tree, fulls);
    return { tree, fulls };
  });
  const [icons] = createResource(
    () => treeData().fulls,
    (fulls) =>
      fulls.length ? backend().pageIcons(fulls) : Promise.resolve({} as Record<string, string>)
  );
  const iconOf = (full: string) => (icons() ?? {})[full];
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
              <span class="ns-macro-label">Namespace</span>
              <Show when={iconOf(root.full)}>
                <span class="page-icon">
                  <EmojiText text={iconOf(root.full)!} />
                </span>
              </Show>
              <a class="page-ref" onClick={(e) => { e.stopPropagation(); openPage(root.full, "page"); }}>
                <EmojiText text={root.seg} />
              </a>
            </div>
            <div class="ns-macro-tree">
              <For each={root.children}>
                {(c) => <NsMacroNode node={c} depth={0} icons={icons() ?? {}} />}
              </For>
            </div>
          </div>
        )}
      </For>
    </Show>
  );
}

/** Breadcrumb rows for OG's "Hierarchy" section of page `name`: ONE row per
 *  descendant namespace LEVEL, each row the segment list of a cumulative path.
 *  Levels are synthesized from descendant page NAMES (every prefix below `name`),
 *  so an intermediate namespace with no file of its own still gets a row — matching
 *  OG, where creating `a/b/c` makes page entities for `a/b` and `a`, so its
 *  `get-namespace-pages` returns a row for each level (not just the deepest leaf).
 *  A namespaced leaf with no descendants → one row: its parent namespace path. */
export function namespaceHierarchyRows(allNames: string[], name: string): string[][] {
  const pSegs = name.split("/");
  const prefix = `${name}/`.toLowerCase();
  const byLower = new Map<string, string[]>(); // cumulative-path (lc) → original segs
  for (const n of allNames) {
    if (!n.toLowerCase().startsWith(prefix)) continue;
    const segs = n.split("/");
    for (let k = pSegs.length + 1; k <= segs.length; k++) {
      const sub = segs.slice(0, k);
      const key = sub.join("/").toLowerCase();
      if (!byLower.has(key)) byLower.set(key, sub);
    }
  }
  if (byLower.size) {
    return [...byLower.values()].sort((a, b) =>
      a.join("/").toLowerCase().localeCompare(b.join("/").toLowerCase())
    );
  }
  // Namespaced leaf with no descendants → the parent namespace's path.
  if (name.includes("/")) return [pSegs.slice(0, -1)];
  return [];
}

/** OG's automatic "Hierarchy" section (components/hierarchy.cljs `structures`):
 *  rendered below any non-journal page that participates in a namespace, as a
 *  bulleted list of breadcrumb paths — one per namespace level (see
 *  `namespaceHierarchyRows`). Each segment links to its cumulative path. */
export function NamespaceHierarchy(props: { name: string }): JSX.Element {
  // Was a createResource keyed on the page NAME → it re-pulled the WHOLE page list
  // over IPC on every navigation (this renders below every non-journal page). Now a
  // pure CPU scan over the shared, epoch-keyed page list (src/pages.ts): recomputes
  // on nav, but with zero IPC and no whole-graph deserialize per nav.
  const rows = createMemo(() => namespaceHierarchyRows(allPageNames(), props.name));
  return (
    <Show when={rows().length > 0}>
      <div class="page-hierarchy">
        <div class="references-header">Hierarchy</div>
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
                            onClick={(e) => { e.stopPropagation(); openPage(full(), "page"); }}
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
      </div>
    </Show>
  );
}
