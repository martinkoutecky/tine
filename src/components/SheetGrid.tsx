import { For, Show, createMemo, type JSX } from "solid-js";
import { doc, formatForBlock } from "../store";
import { AstBody } from "../render/body";
import { facetsOf } from "../render/facets";
import { sheetConfig } from "../sheet/config";
import { buildMatrix, type MatrixCell } from "../sheet/matrix";

const MAX_GRID_DEPTH = 5;

function configForBlock(id: string) {
  const node = doc.byId[id];
  return sheetConfig(node ? facetsOf(node.raw, formatForBlock(id)).properties : []);
}

function blockChildren(id: string): string[] {
  return doc.byId[id]?.children ?? [];
}

export function SheetGrid(props: { id: string }): JSX.Element {
  return <SheetGridInner id={props.id} depth={0} />;
}

function SheetGridInner(props: { id: string; depth: number }): JSX.Element {
  const config = createMemo(() => configForBlock(props.id));
  const rows = createMemo(() =>
    blockChildren(props.id).map((id) => ({
      id,
      cellIds: blockChildren(id),
    }))
  );
  const matrix = createMemo(() => buildMatrix(rows()));
  const columns = createMemo(() => {
    const widths = config().colWidths;
    const tracks: string[] = [];
    for (let col = 0; col < matrix().cols; col++) {
      const px = widths.get(col);
      tracks.push(px == null ? "max-content" : `${px}px`);
    }
    return tracks.join(" ");
  });

  return (
    <Show when={props.depth < MAX_GRID_DEPTH} fallback={<SheetOutline ids={blockChildren(props.id)} depth={props.depth} />}>
      <Show when={rows().length > 0} fallback={<div class="sheet-grid sheet-empty">empty grid</div>}>
        <div class="sheet-grid" style={{ "grid-template-columns": columns() }}>
          <For each={matrix().cells}>
            {(cell) => <SheetGridCell cell={cell} header={config().header && cell.row === 0} depth={props.depth} />}
          </For>
        </div>
      </Show>
    </Show>
  );
}

function SheetGridCell(props: { cell: MatrixCell; header: boolean; depth: number }): JSX.Element {
  return (
    <Show
      when={props.cell.blockId}
      fallback={<div class="sheet-cell sheet-hole" data-row={props.cell.row} data-col={props.cell.col} />}
    >
      {(blockId) => (
        <div
          class="sheet-cell"
          classList={{ "sheet-header-cell": props.header }}
          data-block-id={blockId()}
          data-row={props.cell.row}
          data-col={props.cell.col}
        >
          <SheetBlock id={blockId()} depth={props.depth + 1} />
        </div>
      )}
    </Show>
  );
}

function SheetBlock(props: { id: string; depth: number }): JSX.Element {
  const node = () => doc.byId[props.id];
  const fmt = () => formatForBlock(props.id);
  const facets = createMemo(() => (node() ? facetsOf(node().raw, fmt()) : null));
  const config = createMemo(() => (facets() ? sheetConfig(facets()!.properties) : null));
  const children = () => node()?.children ?? [];

  return (
    <Show when={node()}>
      {(n) => (
        <>
          <div class="sheet-cell-body">
            <AstBody raw={n().raw} format={fmt()} headingLevel={facets()?.headingLevel ?? null} />
          </div>
          <Show when={children().length > 0 || config()?.view === "grid"}>
            <Show
              when={config()?.view === "grid"}
              fallback={<SheetOutline ids={children()} depth={props.depth} />}
            >
              <SheetGridInner id={props.id} depth={props.depth} />
            </Show>
          </Show>
        </>
      )}
    </Show>
  );
}

function SheetOutline(props: { ids: readonly string[]; depth: number }): JSX.Element {
  return (
    <div class="sheet-nested-lines">
      <For each={props.ids}>
        {(id) => (
          <div class="sheet-nested-line" style={{ "padding-left": `${Math.max(0, props.depth) * 14}px` }}>
            <SheetBlock id={id} depth={props.depth + 1} />
          </div>
        )}
      </For>
    </div>
  );
}
