import { For, Show, createEffect, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { dispatchFocusedEditorCommand, blurFocusedEditor, focusedEditorCommandBridge, type MobileEditorCommandId } from "../editorCommandBridge";
import { runGlobalCommand } from "../keybindings";
import { isMobilePlatform } from "../nativeChrome";

type ToolbarIcon =
  | "outdent"
  | "indent"
  | "move-up"
  | "move-down"
  | "soft-newline"
  | "todo"
  | "undo"
  | "redo"
  | "date"
  | "page-ref"
  | "block-ref"
  | "slash";

type ToolbarAction =
  | { label: string; icon: ToolbarIcon; kind: "editor"; command: MobileEditorCommandId }
  | { label: string; icon: ToolbarIcon; kind: "global"; command: "editor/undo" | "editor/redo" };

const ACTIONS: ToolbarAction[] = [
  { label: "Outdent", icon: "outdent", kind: "editor", command: "editor/outdent" },
  { label: "Indent", icon: "indent", kind: "editor", command: "editor/indent" },
  { label: "Move block up", icon: "move-up", kind: "editor", command: "editor/move-block-up" },
  { label: "Move block down", icon: "move-down", kind: "editor", command: "editor/move-block-down" },
  { label: "Soft newline", icon: "soft-newline", kind: "editor", command: "editor/soft-newline" },
  { label: "TODO", icon: "todo", kind: "editor", command: "editor/cycle-todo" },
  // Camera (photo capture) + mic (voice memo) are deferred to P1 — they need
  // native capture intents, not the file-picker upload this once wrapped. See
  // docs/BACKLOG.md. The `editor/upload-asset` bridge command stays available.
  { label: "Undo", icon: "undo", kind: "global", command: "editor/undo" },
  { label: "Redo", icon: "redo", kind: "global", command: "editor/redo" },
  { label: "Date picker", icon: "date", kind: "editor", command: "editor/open-date-picker" },
  { label: "Page reference", icon: "page-ref", kind: "editor", command: "editor/insert-page-ref" },
  { label: "Block reference", icon: "block-ref", kind: "editor", command: "editor/insert-block-ref" },
  { label: "Slash menu", icon: "slash", kind: "editor", command: "editor/open-slash-menu" },
];

const KEYBOARD_GAP_THRESHOLD = 48;

function activeEditorHasFocus(): boolean {
  const el = document.activeElement;
  return el instanceof HTMLTextAreaElement && el.classList.contains("block-editor");
}

function viewportKeyboardTop(): number {
  const vv = window.visualViewport;
  return vv ? vv.height + vv.offsetTop : window.innerHeight;
}

function Icon(props: { name: ToolbarIcon }): JSX.Element {
  switch (props.name) {
    case "outdent":
      return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 6h14M5 12h8M5 18h14M15 9l-3 3 3 3" /></svg>;
    case "indent":
      return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 6h14M11 12h8M5 18h14M6 9l3 3-3 3" /></svg>;
    case "move-up":
      return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 18V6M7 11l5-5 5 5M6 20h12" /></svg>;
    case "move-down":
      return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 6v12M7 13l5 5 5-5M6 4h12" /></svg>;
    case "soft-newline":
      return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M17 5v5a4 4 0 0 1-4 4H6M9 10l-4 4 4 4" /></svg>;
    case "todo":
      return <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="4" y="4" width="16" height="16" rx="3" /><path d="M8 12l3 3 5-7" /></svg>;
    case "undo":
      return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M9 7H5v4M5 11a7 7 0 1 0 2-5" /></svg>;
    case "redo":
      return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M15 7h4v4M19 11a7 7 0 1 1-2-5" /></svg>;
    case "date":
      return <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="4" y="5" width="16" height="15" rx="2" /><path d="M8 3v4M16 3v4M4 10h16" /></svg>;
    case "page-ref":
      return <span class="mobile-keyboard-toolbar-text" aria-hidden="true">[[ ]]</span>;
    case "block-ref":
      return <span class="mobile-keyboard-toolbar-text" aria-hidden="true">(( ))</span>;
    case "slash":
      return <span class="mobile-keyboard-toolbar-text mobile-keyboard-toolbar-slash" aria-hidden="true">/</span>;
  }
}

export function MobileKeyboardToolbar(): JSX.Element {
  let toolbarRef: HTMLDivElement | undefined;
  const [dockTop, setDockTop] = createSignal(typeof window !== "undefined" ? window.innerHeight : 0);
  const [keyboardVisible, setKeyboardVisible] = createSignal(false);
  const [focusedFallback, setFocusedFallback] = createSignal(false);

  const updateDock = () => {
    const top = viewportKeyboardTop();
    setDockTop(top);
    setKeyboardVisible(window.innerHeight - top > KEYBOARD_GAP_THRESHOLD);
    setFocusedFallback(activeEditorHasFocus());
  };

  onMount(() => {
    if (!isMobilePlatform) return;
    const vv = window.visualViewport;
    const updateAfterFocusChange = () => setTimeout(updateDock, 0);
    updateDock();
    vv?.addEventListener("resize", updateDock);
    vv?.addEventListener("scroll", updateDock);
    window.addEventListener("resize", updateDock);
    window.addEventListener("focusin", updateAfterFocusChange);
    window.addEventListener("focusout", updateAfterFocusChange);
    onCleanup(() => {
      vv?.removeEventListener("resize", updateDock);
      vv?.removeEventListener("scroll", updateDock);
      window.removeEventListener("resize", updateDock);
      window.removeEventListener("focusin", updateAfterFocusChange);
      window.removeEventListener("focusout", updateAfterFocusChange);
    });
  });

  createEffect(() => {
    focusedEditorCommandBridge();
    if (isMobilePlatform) queueMicrotask(updateDock);
  });

  // Publish the toolbar's on-screen top as a CSS var so the fixed help "?" FAB
  // (and any other bottom-anchored chrome) can lift ABOVE it instead of
  // overlapping. Re-runs when the dock position or visibility changes; measures
  // the real rendered box (robust to whatever the keyboard does to the viewport).
  createEffect(() => {
    const shown = isMobilePlatform && visible();
    dockTop(); // re-measure when the keyboard/dock moves
    const root = typeof document !== "undefined" ? document.documentElement : null;
    if (!root) return;
    if (!shown) {
      root.style.removeProperty("--mobile-kb-toolbar-lift");
      return;
    }
    requestAnimationFrame(() => {
      if (!toolbarRef) return;
      const top = toolbarRef.getBoundingClientRect().top;
      const lift = Math.max(0, window.innerHeight - top) + 8;
      root.style.setProperty("--mobile-kb-toolbar-lift", `${lift}px`);
    });
  });
  onCleanup(() => {
    if (typeof document !== "undefined") {
      document.documentElement.style.removeProperty("--mobile-kb-toolbar-lift");
    }
  });

  const visible = () =>
    !!focusedEditorCommandBridge() && (keyboardVisible() || focusedFallback());
  const style = () => ({
    top: `calc(${Math.max(0, dockTop())}px - env(safe-area-inset-bottom))`,
  });
  const keepEditorFocus = (e: Event) => e.preventDefault();
  const run = (action: ToolbarAction) => {
    if (action.kind === "global") {
      runGlobalCommand(action.command);
      return;
    }
    dispatchFocusedEditorCommand(action.command);
  };
  const hideKeyboard = (e?: Event) => {
    e?.preventDefault();
    blurFocusedEditor();
  };

  return (
    <Show when={isMobilePlatform && visible()}>
      <div
        ref={toolbarRef}
        class="mobile-keyboard-toolbar"
        data-mobile-keyboard-toolbar
        role="toolbar"
        aria-label="Editor toolbar"
        style={style()}
      >
        <div class="mobile-keyboard-toolbar-strip" data-lenis-prevent>
          <For each={ACTIONS}>
            {(action) => (
              <button
                type="button"
                class="mobile-keyboard-toolbar-btn"
                title={action.label}
                aria-label={action.label}
                onPointerDown={keepEditorFocus}
                onMouseDown={keepEditorFocus}
                onClick={() => run(action)}
              >
                <Icon name={action.icon} />
              </button>
            )}
          </For>
        </div>
        <button
          type="button"
          class="mobile-keyboard-toolbar-btn mobile-keyboard-toolbar-hide"
          title="Hide keyboard"
          aria-label="Hide keyboard"
          onPointerDown={hideKeyboard}
          onMouseDown={keepEditorFocus}
          onClick={() => hideKeyboard()}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <rect x="3" y="5" width="18" height="10" rx="2" />
            <path d="M7 9h.01M11 9h.01M15 9h.01M8 12h8M8 18l4 3 4-3" />
          </svg>
        </button>
      </div>
    </Show>
  );
}
