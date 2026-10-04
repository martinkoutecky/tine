import { TableWrap } from "./TableWrap";
import { For, Match, Switch, type JSX } from "solid-js";
import { openPageTarget, openPageTargetInNewTab } from "../router";
import { openPageInSidebar, openPageContextMenu } from "../ui";
import { shouldOpenTextContextMenu } from "../contextMenuPolicy";
import { formatForPage } from "../document";
import { InlineText } from "../render/inline";
import { openRouteInOtherPane } from "../panes";
import { internalLinkAuxClick, internalLinkDest, internalLinkMouseDown } from "../linkGesture";
import { cellText, type Row } from "./queryMacroSupport";

/** The legacy `:table? true` list view of a query's block results (OG `query-table`): a
 *  plain sortable table of Content / Page / property columns. */
export function QueryLegacyTable(props: {
  cols: string[];
  rows: Row[];
  sortBy: (column: string) => void;
  arrow: (column: string) => string;
}): JSX.Element {
  return (
  <TableWrap>
  <table class="md-table query-table">
    <thead>
      <tr onClick={(e) => e.stopPropagation()}>
        <For each={props.cols}>
          {(c) => (
            <th onClick={() => props.sortBy(c)}>
              {c === "block" ? "Content" : c === "page" ? "Page" : c}{props.arrow(c)}
            </th>
          )}
        </For>
      </tr>
    </thead>
    <tbody>
      <For each={props.rows}>
        {(r) => (
          <tr>
            <For each={props.cols}>
              {(c) => (
                <Switch fallback={<td>{cellText(r, c)}</td>}>
                  <Match when={c === "block"}>
                    <td>
                      <InlineText text={r.text} format={formatForPage(r.page)} />
                    </td>
                  </Match>
                  <Match when={c === "page"}>
                    <td
                      class="qt-page"
                      onClick={(e) => {
                        e.stopPropagation();
                        const target = { name: r.page, pageKind: r.kind, ...(r.path ? { path: r.path } : {}) };
                        const dest = internalLinkDest(e);
                        if (dest === "sidebar") openPageInSidebar(target);
                        else if (dest === "background") openPageTargetInNewTab(target);
                        else if (dest === "pane") openRouteInOtherPane({ kind: "page", ...target });
                        else openPageTarget(target);
                      }}
                      onMouseDown={internalLinkMouseDown}
                      onAuxClick={(e) => {
                        e.stopPropagation();
                        internalLinkAuxClick(e, () => openPageTargetInNewTab({ name: r.page, pageKind: r.kind, ...(r.path ? { path: r.path } : {}) }));
                      }}
                      onContextMenu={(e) => {
                        if (!shouldOpenTextContextMenu(e.target)) return;
                        e.preventDefault();
                        e.stopPropagation();
                        openPageContextMenu(e.clientX, e.clientY, { name: r.page, pageKind: r.kind, ...(r.path ? { path: r.path } : {}) });
                      }}
                    >
                      {r.page}
                    </td>
                  </Match>
                </Switch>
              )}
            </For>
          </tr>
        )}
      </For>
    </tbody>
  </table>
  </TableWrap>
  );
}
