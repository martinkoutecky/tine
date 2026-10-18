// OG's per-block reference-count badge: shown only when the block is referenced.
// Plain click toggles the referrers panel below the block; shift-click opens the
// block in the sidebar (matching OG and the bullet's shift-click).

import { Show, type JSX } from "solid-js";
import { openDurableBlock } from "../blockRefActions";
import { blockRefCount } from "../blockRefCounts";
import { t } from "../i18n";
import { parserReady } from "../render/parse";

export function BlockRefCountBadge(props: { id: string; hidden?: boolean; open: boolean; toggle: () => void }): JSX.Element {
  return (
    <Show when={parserReady() && blockRefCount(props.id) > 0 && !props.hidden}>
      <a
        class="block-refs-count"
        classList={{ open: props.open }}
        title={t("editor-block-open-block-references-shift-click-sidebar")}
        onClick={(e) => {
          e.stopPropagation();
          if (e.shiftKey) openDurableBlock(props.id, "sidebar");
          else props.toggle();
        }}
      >
        {blockRefCount(props.id)}
      </a>
    </Show>
  );
}
