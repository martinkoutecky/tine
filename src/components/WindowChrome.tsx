import { createSignal, For, Show, type JSX } from "solid-js";
import { getCurrentWindow } from "@tauri-apps/api/window";

// Frameless-window chrome. Native decorations are off (so the toolbar doubles as
// the title bar and we save a row), which means we must supply what the window
// manager normally would: minimize / maximize / close buttons, and — because
// KWin gives an undecorated window no resize border — our own edge/corner grips.
// Window dragging itself is handled by `data-tauri-drag-region` on the toolbar.

// `ResizeDirection` from the API is a string-union *type*, not a runtime value,
// so we just use the literals.
type Dir = "North" | "South" | "East" | "West" | "NorthEast" | "NorthWest" | "SouthEast" | "SouthWest";

// Shared maximize state: the maximize button swaps its glyph, and the resize
// grips hide while maximized (a maximized window can't be edge-resized, and the
// grips would otherwise sit on top of the content's scrollbar at the edge).
const [maximized, setMaximized] = createSignal(false);
export { maximized };

/** Track the window's maximized state. Call once (in Tauri) from App; returns a
 *  cleanup. */
export function installWindowChrome(): () => void {
  const w = getCurrentWindow();
  const sync = () => void w.isMaximized().then(setMaximized).catch(() => {});
  sync();
  let un = () => {};
  void w.onResized(sync).then((u) => (un = u));
  return () => un();
}

/** Minimize / maximize-restore / close cluster for the far right of the toolbar. */
export function WindowControls(): JSX.Element {
  const w = getCurrentWindow();
  return (
    <div class="win-controls">
      <button class="win-btn" title="Minimize" onClick={() => void w.minimize()}>
        <svg viewBox="0 0 12 12" width="11" height="11">
          <line x1="2.5" y1="6" x2="9.5" y2="6" stroke="currentColor" stroke-width="1.1" />
        </svg>
      </button>
      <button
        class="win-btn"
        title={maximized() ? "Restore" : "Maximize"}
        onClick={() => void w.toggleMaximize()}
      >
        <Show
          when={maximized()}
          fallback={
            <svg viewBox="0 0 12 12" width="11" height="11">
              <rect x="2.5" y="2.5" width="7" height="7" fill="none" stroke="currentColor" stroke-width="1.1" />
            </svg>
          }
        >
          <svg viewBox="0 0 12 12" width="11" height="11">
            <rect x="2.5" y="3.5" width="6" height="6" fill="none" stroke="currentColor" stroke-width="1.1" />
            <path d="M4.5 3.5 V2.5 H9.5 V7.5 H8.5" fill="none" stroke="currentColor" stroke-width="1.1" />
          </svg>
        </Show>
      </button>
      <button class="win-btn win-close" title="Close" onClick={() => void w.close()}>
        <svg viewBox="0 0 12 12" width="11" height="11">
          <path d="M3 3 L9 9 M9 3 L3 9" stroke="currentColor" stroke-width="1.1" />
        </svg>
      </button>
    </div>
  );
}

const GRIPS: { dir: Dir; cls: string }[] = [
  { dir: "North", cls: "grip-n" },
  { dir: "South", cls: "grip-s" },
  { dir: "East", cls: "grip-e" },
  { dir: "West", cls: "grip-w" },
  { dir: "NorthWest", cls: "grip-nw" },
  { dir: "NorthEast", cls: "grip-ne" },
  { dir: "SouthWest", cls: "grip-sw" },
  { dir: "SouthEast", cls: "grip-se" },
];

/** Thin invisible edge/corner strips that start a window resize on drag. */
export function ResizeGrips(): JSX.Element {
  const w = getCurrentWindow();
  return (
    <For each={GRIPS}>
      {(g) => (
        <div
          class={`resize-grip ${g.cls}`}
          onMouseDown={(e) => {
            if (e.button !== 0) return;
            e.preventDefault();
            void w.startResizeDragging(g.dir);
          }}
        />
      )}
    </For>
  );
}
