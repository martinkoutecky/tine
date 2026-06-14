import { For, Show, Switch, Match, createResource, type JSX } from "solid-js";
import { backend } from "../backend";
import { openPage } from "../router";
import { RefBlocks } from "./RefBlocks";

const ADVANCED_RE = /\[\s*:find|:where|:find/;

// A {{query ...}} block: runs the query and renders matching blocks.
export function QueryMacro(props: { body: string }): JSX.Element {
  const arg = () => props.body.replace(/^query\s*/i, "").trim();
  const [groups] = createResource(arg, (q) => backend().runQuery(q));
  const total = () => groups()?.reduce((a, g) => a + g.blocks.length, 0) ?? 0;

  return (
    <div class="query-block">
      <Switch>
        <Match when={ADVANCED_RE.test(arg())}>
          <div class="query-unsupported">
            Advanced query not supported yet: <code>{`{{${props.body}}}`}</code>
          </div>
        </Match>
        <Match when={true}>
          <div class="query-header">
            Query <span class="query-count">{total()}</span>
          </div>
          <Show
            when={groups() && groups()!.length > 0}
            fallback={<div class="query-empty">No results</div>}
          >
            <For each={groups()}>
              {(g) => (
                <div class="query-group">
                  <div class="query-page" onClick={() => openPage(g.page, g.kind)}>
                    {g.page}
                  </div>
                  <RefBlocks blocks={g.blocks} />
                </div>
              )}
            </For>
          </Show>
        </Match>
      </Switch>
    </div>
  );
}

// A {{embed ((uuid))}} or {{embed [[Page]]}} block.
export function EmbedMacro(props: { body: string }): JSX.Element {
  const target = () => props.body.replace(/^embed\s*/i, "").trim();

  const [data] = createResource(target, async (t) => {
    const blockRef = /^\(\(([^)]+)\)\)$/.exec(t);
    if (blockRef) {
      const g = await backend().resolveBlock(blockRef[1]);
      return g ? { label: g.page, blocks: g.blocks } : null;
    }
    const pageRef = /^\[\[([^\]]+)\]\]$/.exec(t);
    if (pageRef) {
      const p = await backend().getPage(pageRef[1], "page");
      return p ? { label: p.name, blocks: p.blocks } : null;
    }
    return null;
  });

  return (
    <div class="embed-block">
      <Show when={data()} fallback={<div class="embed-missing">{`{{${props.body}}}`}</div>}>
        <RefBlocks blocks={data()!.blocks} />
      </Show>
    </div>
  );
}
