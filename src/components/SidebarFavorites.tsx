import { t } from "../i18n";
// The left sidebar's Favorites list (family 22): one flat run of rows in
// pre-order, indented by depth. The visible index IS `data-row-index`, so a
// drop found by elementFromPoint needs no translation, and a label row and a
// favorite row drag on exactly the same terms.
import { For, Show, createResource, createSignal, type JSX } from "solid-js";
import { resolveBlockBatched, blockRefTarget } from "../resolveBatch";
import { dataRev, graphEpoch } from "../graphSession";
import { readOr } from "../resourceRead";
import { visibleBody } from "../render/block";
import { openPageTarget, replaceActiveRoute, openInNewTab } from "../router";
import { openRouteInOtherPane } from "../panes";
import { openBlockInSidebar, openBlockRefContextMenu } from "../ui";
import { internalLinkAuxClick, internalLinkDest, internalLinkMouseDown } from "../linkGesture";
import {
  addFavoriteGroup, deleteFavoriteGroup, favoritesLayout, moveFavoriteRow,
  renameFavoriteGroup, setFavoriteRowCollapsed,
} from "../favorites";
import { type FavRow, isWithin, itemKind, resolveDrop, visibleRows } from "../favoritesLayout";
import { SidebarTitle } from "./SidebarTitle";
import { pageIcon } from "../pageIconBatch";
import type { PageKind } from "../types";
import { beginRowReorderDrag, rowReorderClickSuppressed, type RowDropTarget } from "./rowReorder";
import "../styles/favorites.css";

/** Nesting step in px; also one pointer "step" right when choosing a depth. */
const INDENT_PX = 16;
const [dropTarget, setDropTarget] = createSignal<RowDropTarget | null>(null);
const [dropDepth, setDropDepth] = createSignal(0);

/** Resolve a drop at `target` for the row at `from`, over the rows that remain
 *  once the dragged subtree is lifted out. Depth is the dragged row's own
 *  depth plus one level per indent step the pointer moved right of the grab. */
function resolveFor(rows: FavRow[], from: number, target: RowDropTarget) {
  const origin = rows[from];
  const slot = target.index + (target.before ? 0 : 1);
  const kept = rows.map((row, i) => ({ row, i })).filter(({ row }) => !isWithin(row.path, origin.path));
  const restSlot = kept.filter(({ i }) => i < slot).length;
  return resolveDrop(kept.map(({ row }) => row), restSlot, origin.depth + Math.round(target.dx / INDENT_PX));
}

function startDrag(from: number, event: PointerEvent) {
  if ((event.target as HTMLElement | null)?.closest("button, a, input, [contenteditable=\"true\"]")) return;
  const rows = visibleRows(favoritesLayout());
  beginRowReorderDrag(event, "#sidebar-favorites-list .nav-page",
    (target) => {
      if (target) setDropDepth(resolveFor(rows, from, target).depth);
      setDropTarget(target);
    },
    (target) => {
      const { parent, index } = resolveFor(rows, from, target);
      moveFavoriteRow(rows[from].path, parent, index);
    });
}

/** The favorites tree. `targetName` is used only for the active highlight;
 *  `open` receives the favorite's stored name. Click opens, shift-click opens
 *  in the right sidebar, middle-click opens a new tab. */
export function SidebarFavorites(props: {
  isActive: (name: string) => boolean;
  targetName: (name: string, kind: PageKind) => string;
  open: (name: string, kind: PageKind, gesture: "normal" | "sidebar" | "new-tab" | "pane" | "context", point?: { x: number; y: number }) => void;
  onActiveNavigationComplete?: () => void;
}): JSX.Element {
  return (
    <div id="sidebar-favorites-list">
      <For each={visibleRows(favoritesLayout())}>
        {(row, i) => {
          const [block] = createResource(
            () => row.node.blockId ? `${row.node.blockId}\0${graphEpoch()}\0${dataRev()}` : null,
            () => resolveBlockBatched(row.node.blockId!),
          );
          const source = () => readOr(block, undefined, "favorite block");
          const dropping = () => dropTarget()?.index === i();
          const rowClass = () => ({ "row-drop-before": dropping() && dropTarget()!.before, "row-drop-after": dropping() && !dropTarget()!.before });
          const style = () => ({
            "padding-left": `${6 + row.depth * INDENT_PX}px`,
            ...(dropping() ? { "--fav-drop-indent": `${6 + dropDepth() * INDENT_PX}px` } : {}),
          });
          const toggle = (
            <Show when={row.node.children.length > 0} fallback={<span class="nav-fav-spacer" />}>
              <button type="button" class="nav-fav-group-toggle" aria-expanded={!row.node.collapsed}
                aria-label={row.node.collapsed ? t("sidebar-favorites-expand") : t("sidebar-favorites-collapse")}
                onClick={() => setFavoriteRowCollapsed(row.path, !row.node.collapsed)}>
                <span class="nav-toggle-caret" classList={{ open: !row.node.collapsed }}>▸</span>
              </button>
            </Show>
          );
          const name = row.node.target;
          if (name === null && !row.node.blockId) return (
            <div class="nav-page nav-fav-group" data-row-index={i()} classList={rowClass()} style={style()}
              onPointerDown={(e) => startDrag(i(), e)}>
              {toggle}
              <input class="nav-fav-group-name" size={Math.max(row.node.raw.length, 4)} value={row.node.raw}
                aria-label={t("sidebar-favorites-rename-group", { groupName: String(row.node.raw) })}
                onChange={(e) => renameFavoriteGroup(row.path, e.currentTarget.value)} />
              {/* Deleting a group keeps its favorites (they move up), so no confirm. */}
              <button type="button" class="nav-fav-group-delete" title={t("sidebar-favorites-delete-this-group-what-it-holds-moves-up")}
                aria-label={t("sidebar-favorites-delete-group", { groupName: String(row.node.raw) })} onClick={() => deleteFavoriteGroup(row.path)}>×</button>
            </div>
          );
          const kind = row.node.kind ?? itemKind(name ?? "");
          const title = () => row.node.blockId ? (source()?.blocks[0] ? visibleBody(source()!.blocks[0].raw).join(" ") : row.node.blockId) : name!;
          const open = (gesture: "normal" | "sidebar" | "new-tab" | "pane" | "context", point?: { x: number; y: number }) => {
            if (!row.node.blockId) { props.open(name!, kind, gesture, point); return; }
            const group = source();
            if (!group) return;
            const target = blockRefTarget(row.node.blockId, group);
            const route = { kind: "page" as const, name: target.page, pageKind: target.pageKind, block: target.uuid, ...(target.path ? { path: target.path } : {}) };
            if (gesture === "sidebar") openBlockInSidebar(target);
            else if (gesture === "new-tab") openInNewTab(route);
            else if (gesture === "pane") openRouteInOtherPane(route);
            else if (gesture === "context" && point) openBlockRefContextMenu(point.x, point.y, target.uuid, target.page, target.pageKind, target.path);
            else { openPageTarget(route); replaceActiveRoute(route); props.onActiveNavigationComplete?.(); }
          };
          return (
            <div class="nav-page" data-row-index={i()} classList={{ active: name !== null && props.isActive(props.targetName(name, kind)), ...rowClass() }}
              style={style()}
              onPointerDown={(e) => startDrag(i(), e)}
              onMouseDown={internalLinkMouseDown}
              onClick={(e) => {
                if (rowReorderClickSuppressed()) return;
                const dest = internalLinkDest(e);
                open(dest === "sidebar" ? "sidebar" : dest === "background" ? "new-tab" : dest === "pane" ? "pane" : "normal");
              }}
              onAuxClick={(e) => internalLinkAuxClick(e, () => open("new-tab"))}
              onContextMenu={(e) => {
                e.preventDefault();
                open("context", { x: e.clientX, y: e.clientY });
              }}
            >
              {toggle}
              {/* ⭐ via EmojiText: WebKitGTK's Skia COLRv1 path crashes painting a
                  raw color-emoji glyph on hardened libstdc++ (#29). */}
              <SidebarTitle text={`${name ? pageIcon(name) || "⭐" : "⭐"} ${title()}`} fullTitle={title()} />
            </div>
          );
        }}
      </For>
      <button type="button" class="nav-fav-add-group" onClick={() => addFavoriteGroup()}>{t("sidebar-favorites-new-group")}</button>
    </div>
  );
}
