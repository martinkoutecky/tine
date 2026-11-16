import { For, Show, createEffect, createResource, onCleanup } from "solid-js";
import { backend } from "../backend";
import { t } from "../i18n";
import { graphMeta } from "../graphSession";
import { readOr } from "../resourceRead";
import { ResourceFailure } from "./ResourceFailure";
import { graphNavigationActions, type GraphNavigationActions } from "./Sidebar";
import { graphRowMenuActions } from "./graphRowMenu";
import { canRevealInFileManager } from "../nativeChrome";
import { writeClipboardText } from "../clipboard";
import { reportGraphOpenFailure } from "../graphOpenFailure";
import { ownedWhen, writeOwned } from "../owned";

/** A view of the same registry and operations as the sidebar, never a graph file. */
export function AllGraphs(props: { actions?: GraphNavigationActions } = {}) {
  const [graphs, { refetch }] = createResource(() => backend().listKnownGraphs());
  const rows = () => readOr(graphs, [], "known graphs") ?? [];
  const actions = () => props.actions ?? graphNavigationActions;
  let list!: HTMLDivElement;
  let alive = true;
  let focused = false;
  onCleanup(() => { alive = false; });
  createEffect(() => {
    if (rows().length && !focused) {
      focused = true;
      queueMicrotask(() => { if (alive) list.querySelector<HTMLButtonElement>("[data-graph-open]")?.focus(); });
    }
  });
  const attempt = (run: () => Promise<unknown>) => {
    const retry = () => void run().catch(error => reportGraphOpenFailure(error, retry));
    retry();
  };
  const move = (event: KeyboardEvent) => {
    const buttons = [...list.querySelectorAll<HTMLButtonElement>("[data-graph-open]")];
    const index = buttons.indexOf(event.target as HTMLButtonElement);
    if (index < 0) return;
    const next = event.key === "Home" ? 0 : event.key === "End" ? buttons.length - 1
      : event.key === "ArrowDown" ? Math.min(index + 1, buttons.length - 1)
      : event.key === "ArrowUp" ? Math.max(index - 1, 0) : -1;
    if (next >= 0) { event.preventDefault(); event.stopPropagation(); buttons[next]?.focus(); }
  };
  return <section class="all-graphs" style={{ padding: "24px" }}>
    <h1>{t("command-go-all-graphs")}</h1>
    <p>{t("all-graphs-keyboard-help")}</p>
    <Show when={canRevealInFileManager()}><p>{t("all-graphs-new-window-help")}</p></Show>
    <ResourceFailure of={graphs} what={t("command-go-all-graphs")} onRetry={() => void refetch()} />
    <div ref={list} onKeyDown={move}>
      <For each={rows()}>{graph => <div class="all-graphs-row" role="group" aria-label={graph.name} style={{ "margin-bottom": "16px" }}>
        <button data-graph-open title={graph.display_path ?? graph.path}
          aria-current={graph.path === graphMeta()?.root ? "true" : undefined}
          onClick={event => attempt(() => actions().openKnown(graph.path, event.shiftKey && canRevealInFileManager()))}
          onKeyDown={event => {
            if (event.key === "Enter") {
              event.preventDefault(); event.stopPropagation();
              attempt(() => actions().openKnown(graph.path, event.shiftKey && canRevealInFileManager()));
            }
          }}>{graph.name}</button>
        <div>{graph.display_path ?? graph.path}</div>
        <For each={graphRowMenuActions(graph, {
          openKnown: (path, newWindow) => actions().openKnown(path, newWindow),
          reveal: path => backend().revealKnownGraph(path), copyPath: writeClipboardText,
          forget: async path => {
            const result = await writeOwned(ownedWhen(() => alive), backend().forgetKnownGraph(path));
            if (result.kind === "current") await refetch();
          },
          desktop: canRevealInFileManager(), isCurrent: graph.path === graphMeta()?.root,
        }).filter(action => !action.disabled)}>{action => <button onClick={() => action.run?.()}>{action.label}</button>}</For>
      </div>}</For>
    </div>
    <button onClick={() => attempt(() => actions().openPicked())}>{t("sidebar-open-graph")}</button>
    <button onClick={() => attempt(() => actions().createNew())}>{t("sidebar-new-graph")}</button>
  </section>;
}
