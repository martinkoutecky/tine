import { For, Show, createResource, type JSX } from "solid-js";
import { page, loadPageDto } from "../store";
import { route, openPage } from "../router";
import { backend } from "../backend";
import { Block } from "./Block";
import { LinkedReferences } from "./LinkedReferences";
import { isPropertyLine } from "../render/block";
import { InlineText } from "../render/inline";

// Loads the page named by the current route into the shared editing store.
async function loadForRoute(r: ReturnType<typeof route>): Promise<boolean> {
  if (r.kind === "journals") {
    const js = await backend().journalsDesc(1, 0);
    if (js[0]) loadPageDto(js[0]);
    return !!js[0];
  } else {
    const dto = await backend().getPage(r.name, r.pageKind);
    if (dto) loadPageDto(dto);
    else loadPageDto({ name: r.name, kind: r.pageKind, title: r.name, pre_block: null, blocks: [] });
    return true;
  }
}

export function PageView(): JSX.Element {
  const [loaded] = createResource(route, loadForRoute);

  return (
    <Show when={loaded() && page.loaded} fallback={<div class="page-loading" />}>
      <div class="page">
        <h1
          class="page-title"
          classList={{ "journal-title": page.kind === "journal" }}
          onClick={() => page.kind === "journal" && openPage(page.name, "journal")}
        >
          {page.title}
        </h1>
        <PreBlock />
        <div class="page-blocks">
          <For each={page.roots}>{(id) => <Block id={id} />}</For>
        </div>
        <LinkedReferences name={page.name} />
      </div>
    </Show>
  );
}

function PreBlock(): JSX.Element {
  return (
    <Show when={page.preBlock}>
      <div class="page-properties">
        <For each={page.preBlock!.split("\n").filter(isPropertyLine)}>
          {(line) => {
            const idx = line.indexOf("::");
            const key = line.slice(0, idx).trim();
            const val = line.slice(idx + 2).trim();
            return (
              <div class="prop-row">
                <span class="prop-key">{key}</span>
                <span class="prop-value">
                  <InlineText text={val} />
                </span>
              </div>
            );
          }}
        </For>
      </div>
    </Show>
  );
}
