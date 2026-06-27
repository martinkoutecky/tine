import { For, Show, createResource, createSignal, type JSX } from "solid-js";
import { backend } from "../backend";
import { openPage } from "../router";
import { graphEpoch } from "../ui";
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
  const [tree] = createResource(
    () => graphEpoch(),
    async () => buildNamespaceTree((await backend().listPages()).map((p) => p.name)),
  );
  return (
    <Show when={(tree() ?? []).length > 0}>
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
  const [data] = createResource(
    () => ({ r: props.root, e: graphEpoch() }),
    async ({ r }) => {
      const prefix = `${r}/`.toLowerCase();
      const names = (await backend().listPages())
        .map((p) => p.name)
        .filter((n) => n.toLowerCase().startsWith(prefix));
      const tree = buildNamespaceTree(names);
      const fulls: string[] = [];
      collectFulls(tree, fulls);
      const icons = fulls.length ? await backend().pageIcons(fulls) : {};
      return { tree, icons };
    }
  );
  return (
    <Show
      when={(data()?.tree ?? []).length > 0}
      fallback={<span class="macro">{`{{namespace ${props.root}}}`}</span>}
    >
      <div class="ns-macro">
        <For each={data()!.tree}>{(n) => <NsMacroNode node={n} depth={0} icons={data()!.icons} />}</For>
      </div>
    </Show>
  );
}

/** Direct child pages of a namespace (pages named `<name>/<segment>`). */
export function NamespaceChildren(props: { name: string }): JSX.Element {
  const [children] = createResource(
    () => ({ n: props.name, e: graphEpoch() }),
    async ({ n }) => {
      const prefix = `${n}/`.toLowerCase();
      const all = await backend().listPages();
      const seen = new Set<string>();
      const direct: { name: string; full: string }[] = [];
      for (const p of all) {
        if (!p.name.toLowerCase().startsWith(prefix)) continue;
        const rest = p.name.slice(n.length + 1);
        const seg = rest.split("/")[0];
        const full = `${n}/${seg}`;
        if (!seen.has(full.toLowerCase())) {
          seen.add(full.toLowerCase());
          direct.push({ name: seg, full });
        }
      }
      return direct.sort((a, b) => a.name.localeCompare(b.name));
    }
  );
  return (
    <Show when={(children() ?? []).length > 0}>
      <div class="namespace-children">
        <div class="references-header">Namespace</div>
        <For each={children()}>
          {(c) => (
            <div class="ns-child" onClick={() => openPage(c.full, "page")}>
              {c.name}
            </div>
          )}
        </For>
      </div>
    </Show>
  );
}
