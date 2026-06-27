import { For, Show, type JSX } from "solid-js";
import {
  tabs,
  activeId,
  setActiveTab,
  closeTab,
  togglePin,
  reorderTab,
  routeTitle,
  tabRoute,
  type Route,
} from "../router";
import { doc } from "../store";
import { splitProps, isBuiltinHidden } from "../editor/properties";
import { EmojiText } from "../render/emoji";

const MAX_TITLE = 32;

// A short, plain-text summary of a zoomed-into block, for the tab label. Drops
// the hidden id::/collapsed:: lines, takes the first non-empty line, and strips
// the common markdown decorations so the pill reads like the block's text.
function blockSummary(raw: string): string {
  const { visible } = splitProps(raw, isBuiltinHidden);
  const line =
    visible
      .split("\n")
      .map((s) => s.trim())
      .find((s) => s.length > 0) ?? "";
  const plain = line
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1") // image → alt text
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1") // link → label
    .replace(/\[\[([^\]]+)\]\]/g, "$1") // page ref → name
    .replace(/\(\(([^)]+)\)\)/g, "$1") // block ref → inner
    .replace(/==/g, "") // highlight markers
    .replace(/[*_~`]{1,3}/g, "") // bold / italic / strike / code
    .replace(/^#{1,6}\s+/, "") // markdown heading
    .replace(/\s+/g, " ")
    .trim();
  return plain.length > MAX_TITLE ? plain.slice(0, MAX_TITLE - 1).trimEnd() + "…" : plain;
}

// Tab label: a zoomed-into block shows its (shortened) content; everything else
// shows the page name (falling back to it when the block isn't loaded or empty).
function tabTitle(r: Route): string {
  if (r.kind === "page" && r.block) {
    const n = doc.byId[r.block];
    if (n) {
      const s = blockSummary(n.raw);
      if (s) return s;
    }
  }
  return routeTitle(r);
}

// Tab strip: click to activate, middle-click to close, double-click to pin,
// drag to reorder. Pinned tabs are "sticky": they sort to the left and stay on
// their content — navigating from one (clicking a page/link, Ctrl-K) opens the
// destination in a new foreground tab instead of leaving the pinned view (zoom
// stays in place; middle-click still opens in the background). Closing a pinned
// tab asks first. The whole session persists across launches (see router
// persist/restoreSession). Always shown — even a single tab is rendered, so the
// pill signals that tabs exist (a feature OG Logseq lacks) without costing
// extra vertical space.
export function TabBar(): JSX.Element {
  let dragId: string | null = null;

  return (
    // The tab strip fills the toolbar's middle, so its empty space is the main
    // window-drag handle (the tabs, being children, still click/drag-to-reorder).
    <div class="tab-bar" data-tauri-drag-region>
      <For each={tabs()}>
        {(t) => (
          <div
            class="tab"
            classList={{ active: t.id === activeId(), pinned: t.pinned }}
            draggable={true}
            onMouseDown={(e) => {
              // Stop the double-click (pin) gesture from word-selecting the label
              // — user-select:none alone still flashes a selection in WebKitGTK.
              // Only the 2nd+ click (detail>=2); the single mousedown that starts
              // a drag is left untouched.
              if (e.detail >= 2) e.preventDefault();
            }}
            onDragStart={(e) => {
              dragId = t.id;
              // WebKitGTK won't actually start a drag unless dataTransfer carries
              // something — without this, dragover/drop never fire and reordering
              // silently does nothing.
              e.dataTransfer?.setData("text/plain", t.id);
              if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
            }}
            onDragOver={(e) => {
              e.preventDefault();
              if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
            }}
            onDrop={(e) => {
              e.preventDefault();
              if (dragId) reorderTab(dragId, t.id);
              dragId = null;
            }}
            onDragEnd={() => (dragId = null)}
            onClick={() => setActiveTab(t.id)}
            onDblClick={() => togglePin(t.id)}
            onAuxClick={(e) => {
              if (e.button === 1) {
                e.preventDefault();
                closeTab(t.id);
              }
            }}
            title={
              t.pinned
                ? "Pinned (sticky) — links open in a new tab. Double-click to unpin"
                : "Double-click to pin (sticky)"
            }
          >
            <Show when={t.pinned}>
              {/* The red 📌 — now a Twemoji SVG <img> (render/emoji.tsx), so it
                  shows everywhere, including WebKitGTK where the emoji *font*
                  painted it blank. */}
              <span class="tab-pin">
                <EmojiText text="📌" />
              </span>
            </Show>
            <span class="tab-title">{tabTitle(tabRoute(t))}</span>
            {/* The last tab can't be closed (closeTab keeps one), so hide its ✕. */}
            <Show when={tabs().length > 1}>
              <span
                class="tab-close"
                onClick={(e) => {
                  e.stopPropagation();
                  closeTab(t.id);
                }}
              >
                ✕
              </span>
            </Show>
          </div>
        )}
      </For>
    </div>
  );
}
