import { Show } from "solid-js";
import { t } from "../i18n";
import { blockPageReadOnly } from "../document";
import { openDurableBlock } from "../blockRefActions";
import { internalLinkAuxClick, internalLinkDest, internalLinkMouseDown } from "../linkGesture";
import { createBulletHistory } from "../seen/bulletHistory";
import { beginDrag, bulletDragMoved } from "./blockGestures";

/** The outline's bullet and shared clock tooltip. Git dates load only on
 * mouse hover, using the source block's page even inside embeds/references.
 * Work: one page snapshot/IPC per hover, no history work on backend cache hits. */
export function BlockBullet(props: { id: string; owner: string; closed: boolean; order: string | number | null; open: () => void }) {
  const history = createBulletHistory(() => props.id,
    () => t("editor-block-click-to-zoom-shift-click-sidebar-ctrl-cmd-click-or-middle-click-new-tab-alt-click-other-pane-drag-to-move"));
  return <span class="bullet-container" classList={{ "bullet-closed": props.closed, ordered: !!props.order }}
    title={history.nativeTitle()} onMouseEnter={history.enter} onMouseLeave={history.leave}
    onMouseDown={e => {
      internalLinkMouseDown(e);
      if (e.button === 0 && !blockPageReadOnly(props.owner)) beginDrag(props.owner, e);
    }}
    onClick={e => {
      e.stopPropagation();
      if (bulletDragMoved()) return;
      switch (internalLinkDest(e)) {
        case "sidebar": openDurableBlock(props.id, "sidebar"); break;
        case "background": openDurableBlock(props.id, "tab"); break;
        case "pane": openDurableBlock(props.id, "pane"); break;
        default: props.open();
      }
    }}
    onAuxClick={e => { if (internalLinkAuxClick(e, () => openDurableBlock(props.id, "tab"))) e.stopPropagation(); }}>
    <Show when={props.order} fallback={<span class="bullet" />}><span class="bullet-order">{props.order}.</span></Show>
    <Show when={history.visible()}><span class="clock-tooltip bullet-tooltip" role="tooltip">{history.text()}</span></Show>
  </span>;
}
